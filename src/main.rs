extern crate hyper;
extern crate toml;

use std::io::{Read, Write};

use hyper::Client;

use hyper::Server;
use hyper::server::Request;
use hyper::server::Response;
use hyper::net::Fresh;

fn main() {
    let address = "0.0.0.0:8080";

    println!("Listening on {}", address);
    Server::http(server).listen(address).unwrap();
}

fn server(_: Request, res: Response<Fresh>) {
    let mut res = res.start().unwrap();
    let symbol_url = &get_config("symbol_urls.public");
    let symbol = &client(symbol_url).to_string();
    let _ = res.write_all(symbol.as_bytes());
    res.end().unwrap();
}

fn client(url: &str) -> String {
    let mut c = Client::new();
    let mut res = c.get(url).send().unwrap();
    let mut body = String::new();
    res.read_to_string(&mut body).unwrap();

    body
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
