//! Drawing of the solid-color fullscreen shm buffer.

use std::ffi::CString;
use std::os::unix::io::BorrowedFd;

use wayland_client::protocol::wl_shm;
use wayland_client::QueueHandle;

use crate::AppState;

impl AppState {
    /// Allocates a memfd, fills it with the solid color and attaches it to the
    /// surface. Prints `VWTEST ready <w>x<h>` on the first successful draw.
    pub fn draw(&self, qh: &QueueHandle<Self>) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        let size = (self.width * self.height * 4) as usize;

        let name = CString::new("vwayland-test-client").unwrap();
        let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
        assert!(fd >= 0, "memfd_create failed");
        assert_eq!(unsafe { libc::ftruncate(fd, size as libc::off_t) }, 0);
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        assert_ne!(ptr, libc::MAP_FAILED, "mmap failed");

        // XRGB8888 = 0x00RRGGBB
        let pixel: u32 = self.color & 0x00ff_ffff;
        let pixels = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u32, size / 4) };
        pixels.fill(pixel);

        let pool = self
            .shm
            .create_pool(unsafe { BorrowedFd::borrow_raw(fd) }, size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            self.width as i32,
            self.height as i32,
            (self.width * 4) as i32,
            wl_shm::Format::Xrgb8888,
            qh,
            (),
        );
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface
            .damage(0, 0, self.width as i32, self.height as i32);
        self.surface.commit();

        buffer.destroy();
        pool.destroy();
        unsafe {
            libc::munmap(ptr, size);
            libc::close(fd);
        }

        if !self.drawn {
            println!("VWTEST ready {}x{}", self.width, self.height);
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
    }
}