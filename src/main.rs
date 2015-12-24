// Copyright 2015 Robert Helmer <rhelmer@rhelmer.org>. See the LICENSE
// file at the top-level directory of this distribution.

/**
  * Symbolapi - a microservice which accepts lists of symbol+addresses, and returns
  * a list of symbolicated functions.
  *
  * The goal is to take an HTTP JSON request such as:
  * {"stacks":[
  *   [
  *     [0,11723767],
  *     [1, 65802]
  *    ]
  *  ],
  *  "memoryMap":[
  *    ["xul.pdb","44E4EC8C2F41492B9369D6B9A059577C2"],
  *    ["wntdll.pdb","D74F79EB1F8D4A45ABCD2F476CCABACC2"]
  *  ],
  *  "version":4
  * }
  *
  * The symbolapi service then downloads the corresponding symbol files (e.g. from S3), and returns
  * the function names for the corresponding addresses (in the "stacks" array) and returns JSON
  * such as:
  * {"symbolicatedStacks": [
  *   [
  *     "XREMain::XRE_mainRun() (in xul.pdb)",
  *     "KiUserCallbackDispatcher (in wntdll.pdb)"]
  *   ],
  *   "knownModules": [true, true]
  * }
  */

extern crate breakpad_symbols;
extern crate hyper;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate rustc_serialize;
extern crate toml;

use std::collections::HashMap;
use std::fs::{File, create_dir_all};
use std::io::{Read, Write};
use std::path::{PathBuf};
use std::thread;

use breakpad_symbols::SymbolFile;
use hyper::Client;
use hyper::server::{Server, Request, Response};
use hyper::status::StatusCode;
use rustc_serialize::json;

/**
  * Incoming JSON request format.
  */
// required JSON keys are non-snakecase
#[allow(non_snake_case)]
#[derive(RustcDecodable)]
pub struct SymbolRequest {
    memoryMap: Vec<(String,String)>,
    // index, offset
    stacks: Vec<Vec<(i8,u64)>>,
    version: u8,
}

/**
  * Outgoing JSON response format.
  */
// required JSON keys are non-snakecase
#[allow(non_snake_case)]
#[derive(RustcEncodable)]
pub struct SymbolResponse {
    symbolicatedStacks: Vec<Vec<String>>,
    knownModules: Vec<bool>,
}

fn main() {
    log4rs::init_file("config/log.toml", Default::default()).unwrap();

    let address = "0.0.0.0:8080";

    info!("Listening on {}", address);
    Server::http(address).unwrap().handle(server).unwrap();
}

/**
  * Receives single HTTP requests and demuxes to symbols file fetches from S3 bucket.
  */
fn server(mut req: Request, mut res: Response) {
    info!("incoming connection from {}", req.remote_addr);

    match req.method {
        hyper::Post => {
            let mut res = res.start().unwrap();

            let mut buffer = String::new();
            let _ = req.read_to_string(&mut buffer);
            debug!("raw POST: {:?}", &buffer);

            let decoded: SymbolRequest = json::decode(&buffer).unwrap();

            debug!("decoded memoryMap: {:?}", decoded.memoryMap);
            debug!("decoded stacks: {:?}", decoded.stacks);
            debug!("decoded version: {}", decoded.version);

            let symbol_url = get_config("symbol_urls.public");

            // stacks come in as an array, turn into hashmap
            let mut stack_map: HashMap<i8, Vec<u64>> = HashMap::new();

            debug!("decoded.stacks: {:?}", decoded.stacks);

            let stacks = decoded.stacks[0].clone();
            for stack in &stacks {
                let (index, offset) = *stack;

                let mut offsets = vec!();
                if stack_map.contains_key(&index) {
                    offsets = stack_map.get(&index).unwrap().clone();
                }
                offsets.push(offset);
                stack_map.insert(index, offsets);
            }

            debug!("stack_map: {:?}", stack_map);

            let symbol_response = client(symbol_url, decoded.memoryMap, stack_map.clone());
            let _ = res.write_all(symbol_response.as_bytes());

            res.end().unwrap();
        },
        _ => { *res.status_mut() = StatusCode::MethodNotAllowed },
    }

    debug!("finished serving request");
}

/**
  * Creates multiple client connections and aggregates result.
  */
fn client(url: String, memory_map: Vec<(String,String)>, stack_map: HashMap<i8, Vec<u64>>) -> String {
    let mut handles = vec![];
    let mut counter: i8 = 0;
    for (debug_file, debug_id) in memory_map {
        let debug_file_name = debug_file.clone();
        let pdb = debug_file.find(".pdb").unwrap();
        let (symbol_name, _) = debug_file.split_at(pdb);
        let symbol_file = format!("{}.sym", symbol_name);
        let this_url = format!("{}/{}/{}/{}", url, debug_file, debug_id, symbol_file);

        let mut symbol_path = PathBuf::new();
        // TODO get from config
        symbol_path.push("testdata/symbols");
        symbol_path.push(&debug_file);
        symbol_path.push(&debug_id);
        symbol_path.push(&symbol_file);

        let stack_map_copy = stack_map.clone();

        let _ = create_dir_all(&symbol_path.parent().unwrap()).unwrap();

        // TODO most of the time is probably spent waiting on I/O, maybe async would be more appropriate?
        // TODO decide min/max possible threads, possibly based on number of cores?
        handles.push(thread::spawn(move || {
            // FIXME use Arc<Mutex<File>> to prevent concurrent writes
            // TODO only write contents if server version newer
            if !&symbol_path.exists() {

                let mut body = String::new();
                let c = Client::new();
                let mut res = c.get(&this_url).send().unwrap();
                let _ = res.read_to_string(&mut body);

                let mut f = File::create(&symbol_path).unwrap();
                let _ = f.write_all(body.as_bytes());
            }

            debug!("symbol_path: {:?}", &symbol_path);
            // FIXME this is having problems with CRLF
            let symbol_provider = match SymbolFile::from_file(&symbol_path) {
                Ok(x) => x,
                Err(x) => panic!("symbol provider failed: symbol_path: {:?}, {}", &symbol_path, x),
            };

            let mut symbols = vec!();
            for stacks in stack_map_copy.get(&counter) {
                for address in stacks {
                    //debug!("attempt to symbolicate: {} for: {:?}", *address, &symbol_path);
                    match symbol_provider.functions.lookup(*address) {
                        Some(x) => symbols.push(x.name.clone()),
                        // return the address rather than function name if symbol not found
                        None => symbols.push(format!("0x{:x}", address)),
                    }
                }
            }
            //debug!("debug_file: {:?}, symbols: {:?}", debug_file, symbols);

            (debug_file_name, symbols)
        }));

        counter += 1;
    }

    let mut result = SymbolResponse {
        symbolicatedStacks: vec!(),
        knownModules: vec!(),
    };

    for handle in handles {
        let (debug_file_name, symbols) = handle.join().unwrap();

        for symbol in symbols {
            result.symbolicatedStacks.push(
                vec!(format!("{} (in {}))", symbol, debug_file_name))
            );
        }
        result.knownModules.push(true);
    }

    json::encode(&result).unwrap()
}

/**
  * Returns individual values from the configuration file.
  */
fn get_config(value_name: &str) -> String {
    // TODO move to actual file, static str for the moment
    let toml: &'static str = r#"
        [symbol_urls]
        public = "https://s3-us-west-2.amazonaws.com/org.mozilla.crash-stats.symbols-public/v1"
    "#;
    // TODO support multiple URLs
    let value: toml::Value = toml.parse().unwrap();

    value.lookup(value_name).unwrap().as_str().unwrap().to_string()
}
