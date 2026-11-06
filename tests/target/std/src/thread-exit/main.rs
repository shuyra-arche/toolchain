fn main() {
    std::thread::spawn(|| {
        std::process::exit(23);
    });
    loop {
        std::thread::yield_now();
    }
}
