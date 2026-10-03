// diag/replica/marker.rs -- unsigned target exe for the QF measurement:
// creates the file given as argument 1 (the "marker") and exits.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() > 1 {
        let _ = std::fs::write(&a[1], b"marker");
    }
}
