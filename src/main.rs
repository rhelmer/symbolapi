// Copyright 2015 Robert Helmer <rhelmer@rhelmer.org>. See the LICENSE
// file at the top-level directory of this distribution.

//! # SymbolAPI
//!
//! SymbolAPI is a microservice which accepts lists of debug symbol names + memory addresses, and
//! returns a list of symbolicated function names.
//!

/// The goal is to take an HTTP JSON request such as:
/// ```
/// {"stacks":[
///   [
///     [0,11723767],
///     [1, 65802]
///    ]
///  ],
///  "memoryMap":[
///    ["xul.pdb","44E4EC8C2F41492B9369D6B9A059577C2"],
///    ["wntdll.pdb","D74F79EB1F8D4A45ABCD2F476CCABACC2"]
///  ],
///  "version":4
/// }
/// ```
/// The symbolapi service then downloads the corresponding symbol files (e.g. from S3), and returns
/// the function names for the corresponding addresses (in the "stacks" array) and returns JSON
/// such as:
/// ```
/// {"symbolicatedStacks": [
///   [
///     "XREMain::XRE_mainRun() (in xul.pdb)",
///     "KiUserCallbackDispatcher (in wntdll.pdb)"]
///   ],
///   "knownModules": [true, true]
/// }
/// ```

extern crate breakpad_symbols;
extern crate flate2;
extern crate hyper;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate rustc_serialize;
extern crate toml;

use std::collections::HashMap;
use std::env;
use std::fs::{File, create_dir_all};
use std::io::{Read, Write};
use std::path::{PathBuf};
use std::thread;

use breakpad_symbols::{Symbolizer, SimpleSymbolSupplier};
use flate2::read::GzDecoder;
use hyper::Client;
use hyper::header::{ContentEncoding, Encoding};
use hyper::server::{Server, Request, Response};
use hyper::status::StatusCode;
use rustc_serialize::json;

/// Incoming JSON request format.
// required JSON keys are non-snakecase
#[allow(non_snake_case)]
#[derive(RustcDecodable)]
#[derive(Debug)]
pub struct SymbolRequest {
    pub memoryMap: Vec<(String,String)>,
    // index, offset
    pub stacks: Vec<Vec<(i8,u64)>>,
    pub version: u8,
}

/// Outgoing JSON response format.
// required JSON keys are non-snakecase
#[allow(non_snake_case)]
#[derive(RustcEncodable)]
#[derive(Debug)]
pub struct SymbolResponse {
    pub symbolicatedStacks: Vec<Vec<String>>,
    pub knownModules: Vec<bool>,
}

fn main() {
    log4rs::init_file("config/log.toml", Default::default()).unwrap();

    let default_port = 5000;
    let port;
    match env::var("PORT") {
        Ok(val) => port = val,
        Err(_) => {
            println!("$PORT unset, using default {}", default_port);
            port = format!("{}", default_port);
        }
    }

    let address = &*format!("0.0.0.0:{}", port);

    info!("Listening on {}", address);
    Server::http(address).unwrap().handle(server).unwrap();
}

/// Stacks come in as a JSON array, but really is a hash map of `<index, address>`
/// where `index` is the position of the `memoryMap` JSON result.
pub fn stacks_to_stack_map(decoded_stacks: Vec<Vec<(i8,u64)>>) -> HashMap<i8, Vec<u64>> {
    debug!("decoded_stacks: {:?}", decoded_stacks);

    let mut stack_map: HashMap<i8, Vec<u64>> = HashMap::new();
    for stack in &decoded_stacks[0] {
        let (index, offset) = *stack;

        let mut offsets = vec!();
        if stack_map.contains_key(&index) {
            offsets = stack_map.get(&index).unwrap().clone();
        }
        offsets.push(offset);
        &stack_map.insert(index, offsets);
    }

    debug!("stack_map: {:?}", stack_map);
    stack_map
}

/// Receives single HTTP requests and demuxes to symbols file fetches from S3 bucket via the
/// client function.
pub fn server(mut req: Request, mut res: Response) {
    info!("incoming connection from {}", req.remote_addr);

    match req.method {
        hyper::Post => {
            let mut res = res.start().unwrap();

            let mut buffer = String::new();
            req.read_to_string(&mut buffer).unwrap();
            debug!("raw POST: {:?}", &buffer);

            let decoded: SymbolRequest = json::decode(&buffer).unwrap();

            debug!("decoded memoryMap: {:?}", decoded.memoryMap);
            debug!("decoded stacks: {:?}", decoded.stacks);
            debug!("decoded version: {}", decoded.version);

            let symbol_url = get_config("symbol_urls.public");
            debug!("symbol url: {:?}", symbol_url);
            let stack_map = stacks_to_stack_map(decoded.stacks);

            // FIXME limit the number of possible threads spawned by client()
            // TODO maybe push these into a queue and have a thread pool service the queue?
            let symbol_response: SymbolResponse = client(symbol_url, decoded.memoryMap, stack_map);
            let json_response = match json::encode(&symbol_response) {
                Ok(x) => x,
                Err(x) => panic!("cannot JSON encode {:?}, {:?}", symbol_response, x)
            };
            res.write_all(json_response.as_bytes()).unwrap();

            res.end().unwrap();
        },
        hyper::Get => {
            let mut res = res.start().unwrap();
            res.write_all("symbolapi, see github.com/rhelmer/symbolapi".as_bytes()).unwrap();
        },
        _ => { *res.status_mut() = StatusCode::MethodNotAllowed },
    }

    debug!("finished serving request");
}

/// Creates multiple client connections to fetch debug symbols from S3 bucket and symbolicate using
/// memory and stack maps, aggregates results and returns SymbolResult.
pub fn client(url: String, memory_map: Vec<(String,String)>, stack_map: HashMap<i8, Vec<u64>>) -> SymbolResponse {
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

        let mut full_symbol_path = symbol_path.clone();
        full_symbol_path.push(&debug_file);
        full_symbol_path.push(&debug_id);
        full_symbol_path.push(&symbol_file);

        let stack_map_copy = stack_map.clone();

        create_dir_all(&full_symbol_path.parent().unwrap()).unwrap();

        let supplier = SimpleSymbolSupplier::new(vec!(symbol_path.clone()));

        // TODO most of the time is probably spent waiting on I/O, maybe async would be more appropriate?
        // TODO decide min/max possible threads, possibly based on number of cores?
        handles.push(thread::spawn(move || {

            let symbolizer = Symbolizer::new(supplier);
            // FIXME use Arc<Mutex<File>> to prevent concurrent writes
            // TODO only write contents if server version newer
            if !&full_symbol_path.exists() {

                let c = Client::new();
                let mut res = c.get(&this_url).send().unwrap();
                let is_gzipped = match res.headers.get::<ContentEncoding>() {
                    Some(x) => x.contains(&Encoding::Gzip),
                    None => false,
                };

                let mut body = String::new();
                if is_gzipped {
                    let mut raw_body = vec!();
                    res.read_to_end(&mut raw_body).unwrap();
                    let mut d = match GzDecoder::new(&raw_body[..]) {
                        Ok(x) => x,
                        Err(x) => panic!("cannot gunzip {:?}, {:?}", raw_body, x)
                    };
                    d.read_to_string(&mut body).unwrap();
                } else {
                    res.read_to_string(&mut body).unwrap();
                }

                let mut f = File::create(&full_symbol_path).unwrap();
                f.write_all(body.as_bytes()).unwrap();
            }

            debug!("symbol_path: {:?}", &symbol_path);

            let mut symbols = vec!();
            let mut known_module = true;
            for stacks in stack_map_copy.get(&counter) {
                for address in stacks {
                    debug!("attempt to symbolicate: {} for: {:?}", *address, &full_symbol_path);
                    match symbolizer.get_symbol_at_address(&debug_file_name, &debug_id, *address) {
                        Some(x) => symbols.push(x),
                        // return the address rather than function name if symbol not found
                        None => {
                            symbols.push(format!("0x{:x}", address));
                            known_module = false;
                        },
                    }
                }
            }
            debug!("debug_file_name: {:?}, symbols: {:?}", debug_file_name, symbols);

            (debug_file_name, symbols, known_module)
        }));

        counter += 1;
    }

    let mut result = SymbolResponse {
        symbolicatedStacks: vec!(),
        knownModules: vec!(),
    };

    let mut symbolicated_stacks = vec!();

    // pass anything with index -1 through
    for stacks in stack_map.get(&-1) {
        for address in stacks {
            symbolicated_stacks.push(
                format!("0x{:x}", address)
            );
        }
    }

    for handle in handles {
        let (debug_file_name, symbols, known_module) = handle.join().unwrap();

        for symbol in symbols {
            symbolicated_stacks.push(
                format!("{} (in {})", symbol, debug_file_name)
            );
        }
        result.knownModules.push(known_module);
    }

    // the required result format requires this to be a vec-of-vecs
    result.symbolicatedStacks.push(symbolicated_stacks);

    result
}

/// Returns individual values from the configuration file.
fn get_config(value_name: &str) -> String {
    let mut f = match File::open("config/symbolapi.toml") {
        Ok(x) => x,
        Err(x) => panic!("cannot open config file config/symbolapi.toml: {}", x)
    };
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    let value: toml::Value = match s.parse() {
        Ok(x) => x,
        Err(x) => panic!("cannot parse config file: {:?}", x)
    };

    let result = match value.lookup(value_name) {
        Some(x) => x,
        None => panic!("config value not found for {}", value_name)
    };

    result.as_str().unwrap().to_string()
}
