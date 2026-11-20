//! Classic Bluetooth RFCOMM links over Winsock, with every wait bounded.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::Bluetooth::{AF_BTH, BTHPROTO_RFCOMM, SOCKADDR_BTH};
use windows_sys::Win32::Networking::WinSock::{
    FD_SET, FIONBIO, INVALID_SOCKET, SOCK_STREAM, SOCKADDR, SOCKET, TIMEVAL, WSADATA,
    WSAEWOULDBLOCK, WSAGetLastError, WSAStartup, closesocket, connect, ioctlsocket, recv, select,
    send,
};
use windows_sys::core::GUID;

fn winsock() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        let mut wsa: WSADATA = unsafe { std::mem::zeroed() };
        unsafe { WSAStartup(0x0202, &mut wsa) == 0 }
    })
}

pub struct Link(SOCKET);

impl Drop for Link {
    fn drop(&mut self) {
        unsafe { closesocket(self.0) };
    }
}

impl Link {
    /// Opens `service` on the device; Windows finds the channel over SDP.
    pub fn open(addr: u64, service: GUID, timeout: Duration) -> Option<Self> {
        if !winsock() {
            return None;
        }
        let s = unsafe {
            windows_sys::Win32::Networking::WinSock::socket(
                AF_BTH as i32,
                SOCK_STREAM,
                BTHPROTO_RFCOMM as i32,
            )
        };
        if s == INVALID_SOCKET {
            return None;
        }
        let link = Link(s);
        let mut on = 1u32;
        unsafe { ioctlsocket(s, FIONBIO, &mut on) };
        let sa = SOCKADDR_BTH {
            addressFamily: AF_BTH,
            btAddr: addr,
            serviceClassId: service,
            port: 0,
        };
        let r = unsafe {
            connect(
                s,
                &sa as *const _ as *const SOCKADDR,
                size_of::<SOCKADDR_BTH>() as i32,
            )
        };
        let ok =
            r == 0 || (unsafe { WSAGetLastError() } == WSAEWOULDBLOCK && link.wait(true, timeout));
        ok.then_some(link)
    }

    /// Readable (or connected, for `write`) before the timeout, without an error.
    fn wait(&self, write: bool, timeout: Duration) -> bool {
        let mut set = FD_SET {
            fd_count: 1,
            fd_array: [0; 64],
        };
        set.fd_array[0] = self.0;
        let mut err = FD_SET {
            fd_count: 1,
            fd_array: [0; 64],
        };
        err.fd_array[0] = self.0;
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let tv = TIMEVAL {
            tv_sec: ms / 1000,
            tv_usec: (ms % 1000) * 1000,
        };
        let (r, w) = if write {
            (std::ptr::null_mut(), &mut set as *mut _)
        } else {
            (&mut set as *mut _, std::ptr::null_mut())
        };
        let n = unsafe { select(0, r, w, &mut err, &tv) };
        n > 0 && set.fd_count == 1 && err.fd_count == 0
    }

    pub fn send(&self, data: &[u8]) -> bool {
        unsafe { send(self.0, data.as_ptr(), data.len() as i32, 0) == data.len() as i32 }
    }

    /// Collects the stream until `pick` finds what it wants in it, the link closes, or time runs out.
    pub fn read_until<T>(
        &self,
        timeout: Duration,
        cap: usize,
        mut pick: impl FnMut(&[u8]) -> Option<T>,
    ) -> Option<T> {
        let end = Instant::now() + timeout;
        let mut buf = Vec::new();
        while buf.len() < cap {
            let left = end.checked_duration_since(Instant::now())?;
            if !self.wait(false, left) {
                return None;
            }
            let mut chunk = [0u8; 512];
            let n = unsafe { recv(self.0, chunk.as_mut_ptr(), chunk.len() as i32, 0) };
            if n <= 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n as usize]);
            if let Some(v) = pick(&buf) {
                return Some(v);
            }
        }
        None
    }
}
