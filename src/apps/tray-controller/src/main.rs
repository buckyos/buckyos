#![windows_subsystem = "windows"]

extern "C" {
    fn entry();
}

fn main() {
    unsafe {
        entry();
    }
}
