use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

fn main() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .expect("healthcheck: connect to 127.0.0.1:8080 failed");

    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("healthcheck: set read timeout failed");

    let request = b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(request).expect("healthcheck: write request failed");

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    if response.contains("200 OK") && response.contains("\"status\":\"ok\"") {
        std::process::exit(0);
    }

    eprintln!(
        "healthcheck: unexpected response — {}",
        response.lines().next().unwrap_or("(empty)")
    );
    std::process::exit(1);
}
