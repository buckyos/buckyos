#![windows_subsystem = "windows"]

mod ffi_extern;

extern "C" {
    fn entry();
}

fn main() {
    unsafe {
        entry();
    }
}
