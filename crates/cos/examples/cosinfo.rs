//! Print how a file was opened: revisions, repair notes, trailer. `cargo run -p printcraft-cos --example cosinfo f.pdf`
fn main() {
    let path = std::env::args().nth(1).expect("usage: cosinfo <file.pdf>");
    let bytes = std::sync::Arc::new(std::fs::read(&path).expect("readable"));
    match printcraft_cos::Document::open(bytes) {
        Ok(doc) => {
            println!("version {}  revisions {:?}", doc.version(), doc.revisions());
            for r in doc.repair_log() {
                println!("repair: {r}");
            }
            let mut t = Vec::new();
            printcraft_cos::serialize(&printcraft_cos::Object::Dict(doc.trailer().clone()), &mut t);
            println!("trailer {}", String::from_utf8_lossy(&t));
        }
        Err(e) => println!("error: {e}"),
    }
}
