fn main() {
    std::thread::spawn(|| {
        panic!("worker panic terminates all threads");
    });
    loop {
        std::thread::yield_now();
    }
}
