// Opens a connection, so the import check must refuse it.
fn main() {
    let _ = std::net::TcpStream::connect("127.0.0.1:9");
}
