// Copyright 2015 Robert Helmer <rhelmer@rhelmer.org>. See the LICENSE
// file at the top-level directory of this distribution.

/**
  * Symbolapi - a microservice which accepts lists of symbol+addresses, and returns
  * a list of symbolicated functions.
  */

extern crate breakpad_symbols;
extern crate hyper;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate rustc_serialize;
extern crate toml;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;

use breakpad_symbols::SymbolFile;
use hyper::Client;
use hyper::server::{Server, Request, Response};
use hyper::status::StatusCode;
use rustc_serialize::json;

// required JSON keys are non-snakecase
#[allow(non_snake_case)]
#[derive(RustcDecodable)]
/**
  * Incoming JSON request format.
  */
pub struct SymbolRequest {
    memoryMap: Vec<(String,String)>,
    // TODO check that key actually fits in 8-bit int
    stacks: Vec<Vec<(i8,i64)>>,
    symbolSources: Vec<String>,
    version: u8,
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
    // TODO log IP address
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
            debug!("decoded symbolSources: {:?}", decoded.symbolSources);
            debug!("decoded version: {}", decoded.version);

            let symbol_url = get_config("symbol_urls.public");
            let symbols = client(symbol_url, decoded.memoryMap);
            let _ = res.write_all(symbols.as_bytes());

            res.end().unwrap();
        },
        _ => { *res.status_mut() = StatusCode::MethodNotAllowed },
    }

    debug!("finished serving request");
}

/**
  * Creates multiple client connections and aggregates result.
  */
fn client(url: String, memory_map: Vec<(String,String)>) -> String {
    let mut handles = vec![];

    for (debug_file, debug_id) in memory_map {
        let pdb = debug_file.find(".pdb").unwrap();
        let (symbol_name, _) = debug_file.split_at(pdb);
        let symbol_file = format!("{}.sym", symbol_name);
        let this_url = format!("{}/{}/{}/{}", url, debug_file, debug_id, symbol_file);

        // TODO get from config
        let mut symbol_path = PathBuf::new();
        symbol_path.push("testdata/symbols");
        symbol_path.push(&debug_file);
        symbol_path.push(&debug_id);
        symbol_path.push(&symbol_file);

        // TODO most of the time is spent waiting on I/O, maybe async would be more appropriate?
        // TODO decide min/max possible threads, possibly based on number of cores?
        handles.push(thread::spawn(move || {
            let mut body = String::new();
            let c = Client::new();
            let mut res = c.get(&this_url).send().unwrap();
            let _ = res.read_to_string(&mut body);

            // FIXME fake values, for testing
            let symbol = symbolize(&symbol_path.as_path(), 0x1010);

            match symbol {
                Some(x) => { debug!("{}", x) },
                None => { panic!("Could not symbolicate (...)") },
            }

            (symbol_file, body)
        }));
    }

    let mut result = String::new();

    for handle in handles {
        // TODO stash this file on disk
        let (symbol_file, body) = handle.join().unwrap();

        for c in format!("{:?} {:?}\n", symbol_file, body).chars() {
            result.push(c);
        }
    }

    result
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

/**
  * Symbolicates based on incoming address
  */
fn symbolize(symbol_path: &Path, address: u64) -> Option<String> {
    debug!("symbol_path: {:?}", &symbol_path);
    let sym = SymbolFile::from_file(&symbol_path).unwrap();

    Some(sym.functions.lookup(address).unwrap().name.clone())
}
