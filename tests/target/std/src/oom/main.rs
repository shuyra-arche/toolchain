fn main() {
    std::alloc::handle_alloc_error(std::alloc::Layout::from_size_align(4096, 4096).unwrap());
}
