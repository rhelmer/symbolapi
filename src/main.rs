extern crate hyper;
extern crate toml;

extern crate rustc_serialize;
use rustc_serialize::json;

use std::io::{Read, Write};
use std::thread;
use std::collections::HashMap;

use hyper::Client;
use hyper::server::{Server, Request, Response};
use hyper::status::StatusCode;

// required JSON variables in structs are non-snakecase
#[allow(non_snake_case)]
#[derive(RustcDecodable)]
pub struct SymbolRequest {
    memoryMap: Vec<HashMap<String,String>>,
    stacks: Vec<HashMap<u8,u32>>,
    symbolSources: Vec<String>,
    version: u8,
}

fn main() {
    let address = "0.0.0.0:8080";

    println!("Listening on {}", address);
    Server::http(address).unwrap().handle(server).unwrap();
}

fn server(mut req: Request, mut res: Response) {
    match req.method {
        hyper::Post => {
            let mut buffer = String::new();
            let _ = req.read_to_string(&mut buffer);
            println!("DEBUG raw POST: {:?}", &buffer);
            let decoded: SymbolRequest = json::decode(&buffer).unwrap();
            println!("DEBUG decoded memoryMap: {:?}", decoded.memoryMap);
            println!("DEBUG decoded stacks: {:?}", decoded.stacks);
            println!("DEBUG decoded symbolSources: {:?}", decoded.symbolSources);
            println!("DEBUG decoded version: {}", decoded.version);
        },
        _ => { *res.status_mut() = StatusCode::MethodNotAllowed },
    }
    let mut res = res.start().unwrap();
    let symbol_url = get_config("symbol_urls.public");
    let symbol = client(symbol_url);
    let _ = res.write_all(symbol.as_bytes());
    res.end().unwrap();
}

fn client(url: String) -> String {
    let mut handles = vec![];
    // TODO decide smart min/max possible threads
    for i in 0..5 {
        let this_url = url.clone();
        handles.push(thread::spawn(move || {
            let c = Client::new();
            let mut res = c.get(&this_url).send().unwrap();
            let mut body = String::new();
            let _ = res.read_to_string(&mut body);

            (i, body)
        }));
    }

    let mut result = String::new();

    for handle in handles {
        let (i, body) = handle.join().unwrap();
        for c in format!("{:?} {:?}\n", i, body).chars() {
            result.push(c);
        }
    }

    result
}

fn get_config(value_name: &str) -> String {
    let toml = r#"
        [symbol_urls]
        public = "https://s3-us-west-2.amazonaws.com/org.mozilla.crash-stats.symbols-public/v1/"
    "#;
    // TODO support multiple URLs
    let value: toml::Value = toml.parse().unwrap();

    value.lookup(value_name).unwrap().as_str().unwrap().to_string()
}
