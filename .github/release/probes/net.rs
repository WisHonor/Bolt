// Opens a connection, so the import check must refuse it. On Windows it is also built
// with /DELAYLOAD:ws2_32.dll, which the check must refuse too.
fn main() {
    let _ = std::net::TcpStream::connect("127.0.0.1:9");
}
