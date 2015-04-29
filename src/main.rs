extern crate hyper;

use std::io;

use hyper::server::{Server, Request, Response};
use hyper::Client;
use hyper::status::StatusCode;
use hyper::uri::RequestUri;

fn main() {
    let address = "0.0.0.0:8080";
    println!("Listening on {}", address);
    serve(address);
}

fn serve(address: &str) {
    Server::http(|req: Request, mut res: Response| {
        *res.status_mut() = match (req.method, req.uri) {
            (hyper::Get, RequestUri::AbsolutePath(ref path)) if path == "/" => {
                let url = "https://s3-us-west-2.amazonaws.com/org.mozilla.crash-stats.symbols-public/v1/XUL/7B3E0143CD44393499AC712B4ACD26FC0/XUL.sym";
                fetch(url);
                StatusCode::Ok

            },
            (hyper::Get, _) => StatusCode::NotFound,
            _ => StatusCode::MethodNotAllowed
        };

        res.start().unwrap().end().unwrap();
    }).listen(address).unwrap();
    println!("Listening on {}", address);
}

fn fetch(url: &str) {
    let mut client = Client::new();

    let mut res = match client.get(&*url).send() {
        Ok(res) => res,
        Err(err) => panic!("Failed to connect: {:?}", err)
    };

    println!("Response: {}", res.status);
    println!("Headers:\n{}", res.headers);
    io::copy(&mut res, &mut io::stdout()).unwrap();
}
