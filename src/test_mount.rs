use std::ffi::CString;
use std::process::Command;
use std::os::unix::process::CommandExt;
use std::fs;

fn main() {
    fs::write("test.env", "SECRET=1234").unwrap();

    let mut cmd = Command::new("cat");
    cmd.arg("test.env");

    let source = CString::new("/dev/null").unwrap();
    // Use absolute path for target
    let target = CString::new(std::env::current_dir().unwrap().join("test.env").to_str().unwrap()).unwrap();
    let fs_type = CString::new("none").unwrap();

    unsafe {
        cmd.pre_exec(move || {
            let ret_ns = libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS);
            if ret_ns == 0 {
                libc::mount(
                    source.as_ptr(),
                    target.as_ptr(),
                    fs_type.as_ptr(),
                    libc::MS_BIND,
                    std::ptr::null(),
                );
            }
            Ok(())
        });
    }

    cmd.status().unwrap();
}
