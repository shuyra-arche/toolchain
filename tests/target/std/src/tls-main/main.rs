struct Local;
impl Drop for Local {
    fn drop(&mut self) {
        assert_eq!(std::thread::current().name(), Some("main"));
        panic!("main TLS destructor executed");
    }
}
thread_local! { static LOCAL: Local = const { Local }; }
fn main() {
    LOCAL.with(|_| ());
}
