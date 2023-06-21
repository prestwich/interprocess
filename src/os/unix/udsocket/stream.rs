use super::{
    c_wrappers,
    cmsg::{CmsgMut, CmsgRef},
    util::{make_msghdr_r, make_msghdr_w},
    ToUdSocketPath, UdSocketPath,
};
use crate::{
    os::unix::{unixprelude::*, FdOps},
    TryClone,
};
use libc::{sockaddr_un, SOCK_STREAM};
use std::{
    fmt::{self, Debug, Formatter},
    io::{self, IoSlice, IoSliceMut, Read, Write},
    net::Shutdown,
};
use to_method::To;

/// A Unix domain socket byte stream, obtained either from [`UdStreamListener`](super::UdStreamListener) or by connecting to an existing server.
///
/// # Examples
///
/// ## Basic client
/// ```no_run
/// use interprocess::os::unix::udsocket::UdStream;
/// use std::io::prelude::*;
///
/// let mut conn = UdStream::connect("/tmp/example1.sock")?;
/// conn.write_all(b"Hello from client!")?;
/// let mut string_buffer = String::new();
/// conn.read_to_string(&mut string_buffer)?;
/// println!("Server answered: {}", string_buffer);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
// TODO update with comments and stuff
pub struct UdStream(FdOps);
impl UdStream {
    /// Connects to a Unix domain socket server at the specified path.
    ///
    /// See [`ToUdSocketPath`] for an example of using various string types to specify socket paths.
    ///
    /// # System calls
    /// - `socket`
    /// - `connect`
    pub fn connect<'a>(path: impl ToUdSocketPath<'a>) -> io::Result<Self> {
        Self::_connect(path.to_socket_path()?, false)
    }
    #[cfg(feature = "tokio")]
    pub(crate) fn connect_nonblocking<'a>(path: impl ToUdSocketPath<'a>) -> io::Result<Self> {
        Self::_connect(path.to_socket_path()?, true)
    }
    fn _connect(path: UdSocketPath<'_>, nonblocking: bool) -> io::Result<Self> {
        let addr = path.try_to::<sockaddr_un>()?;

        let fd = c_wrappers::create_uds(SOCK_STREAM, nonblocking)?;
        unsafe {
            // SAFETY: addr is well-constructed
            c_wrappers::connect(fd.0.as_fd(), &addr)?;
        }
        c_wrappers::set_passcred(fd.0.as_fd(), true)?;

        Ok(Self(fd))
    }

    /// Receives both bytes and ancillary data from the socket stream.
    ///
    /// The ancillary data buffer is automatically converted from the supplied value, if possible. For that reason, mutable slices of bytes (`u8` values) can be passed directly.
    ///
    /// # System calls
    /// - `recvmsg`
    #[inline]
    pub fn recv_ancillary(&self, buf: &mut [u8], abuf: &mut CmsgMut<'_>) -> io::Result<(usize, usize)> {
        self.recv_ancillary_vectored(&mut [IoSliceMut::new(buf)], abuf)
    }
    /// Receives bytes and ancillary data from the socket stream, making use of [scatter input] for the main data.
    ///
    /// The ancillary data buffer is automatically converted from the supplied value, if possible. For that reason, mutable slices of bytes (`u8` values) can be passed directly.
    ///
    /// # System calls
    /// - `recvmsg`
    ///
    /// [scatter input]: https://en.wikipedia.org/wiki/Vectored_I/O " "
    pub fn recv_ancillary_vectored(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        abuf: &mut CmsgMut<'_>,
    ) -> io::Result<(usize, usize)> {
        let mut hdr = make_msghdr_r(bufs, abuf)?;

        let (success, bytes_read) = unsafe {
            let result = libc::recvmsg(self.as_raw_fd(), &mut hdr as *mut _, 0);
            (result != -1, result as usize)
        };
        ok_or_ret_errno!(success => (bytes_read, hdr.msg_controllen as _))
    }

    /// Sends bytes and ancillary data into the socket stream.
    ///
    /// The ancillary data buffer is automatically converted from the supplied value, if possible. For that reason, slices and `Vec`s of `AncillaryData` can be passed directly.
    ///
    /// # System calls
    /// - `sendmsg`
    #[inline]
    pub fn send_ancillary(&self, buf: &[u8], abuf: CmsgRef<'_>) -> io::Result<(usize, usize)> {
        self.send_ancillary_vectored(&[IoSlice::new(buf)], abuf)
    }

    /// Sends bytes and ancillary data into the socket stream, making use of [gather output] for the main data.
    ///
    /// The ancillary data buffer is automatically converted from the supplied value, if possible. For that reason, slices and `Vec`s of `AncillaryData` can be passed directly.
    ///
    /// # System calls
    /// - `sendmsg`
    ///
    /// [gather output]: https://en.wikipedia.org/wiki/Vectored_I/O " "
    pub fn send_ancillary_vectored(&self, bufs: &[IoSlice<'_>], abuf: CmsgRef<'_>) -> io::Result<(usize, usize)> {
        let hdr = make_msghdr_w(bufs, abuf)?;

        let (success, bytes_written) = unsafe {
            let result = libc::sendmsg(self.as_raw_fd(), &hdr as *const _, 0);
            (result != -1, result as usize)
        };
        ok_or_ret_errno!(success => (bytes_written, hdr.msg_controllen as _))
    }

    /// Shuts down the read, write, or both halves of the stream. See [`Shutdown`].
    ///
    /// Attempting to call this method with the same `how` argument multiple times may return `Ok(())` every time or it may return an error the second time it is called, depending on the platform. You must either avoid using the same value twice or ignore the error entirely.
    #[inline]
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        c_wrappers::shutdown(self.as_fd(), how)
    }

    /// Enables or disables the nonblocking mode for the stream. By default, it is disabled.
    ///
    /// In nonblocking mode, calls to the `recv…` methods and the [`Read`] trait methods will never wait for at least one byte of data to become available; calls to `send…` methods and the [`Write`] trait methods will never wait for the other side to remove enough bytes from the buffer for the write operation to be performed. Those operations will instead return a [`WouldBlock`](io::ErrorKind::WouldBlock) error immediately, allowing the thread to perform other useful operations in the meantime.
    #[inline]
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        c_wrappers::set_nonblocking(self.as_fd(), nonblocking)
    }
    /// Checks whether the stream is currently in nonblocking mode or not.
    #[inline]
    pub fn is_nonblocking(&self) -> io::Result<bool> {
        c_wrappers::get_nonblocking(self.as_fd())
    }

    /// Fetches the credentials of the other end of the connection without using ancillary data. The returned structure contains the process identifier, user identifier and group identifier of the peer.
    #[cfg(uds_peerucred)]
    #[cfg_attr( // uds_peerucred template
        feature = "doc_cfg",
        doc(cfg(any(
            all(
                target_os = "linux",
                any(
                    target_env = "gnu",
                    target_env = "musl",
                    target_env = "musleabi",
                    target_env = "musleabihf"
                )
            ),
            target_os = "emscripten",
            target_os = "redox",
            target_os = "haiku"
        )))
    )]
    pub fn get_peer_credentials(&self) -> io::Result<libc::ucred> {
        c_wrappers::get_peer_ucred(self.as_fd())
    }
}

/// A list of used system calls is available.
impl Read for &UdStream {
    /// # System calls
    /// - `read`
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&self.0).read(buf)
    }
    /// # System calls
    /// - `readv`
    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        (&self.0).read_vectored(bufs)
    }
}
/// A list of used system calls is available.
impl Read for UdStream {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
    #[inline]
    fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        (&*self).read_vectored(bufs)
    }
}
/// A list of used system calls is available.
impl Write for &UdStream {
    /// # System calls
    /// - `write`
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&self.0).write(buf)
    }
    /// # System calls
    /// - `writev`
    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        (&self.0).write_vectored(bufs)
    }
    /// # System calls
    /// None performed.
    fn flush(&mut self) -> io::Result<()> {
        // You cannot flush a socket
        Ok(())
    }
}
/// A list of used system calls is available.
impl Write for UdStream {
    /// # System calls
    /// - `write`
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }
    /// # System calls
    /// - `writev`
    #[inline]
    fn write_vectored(&mut self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        (&*self).write_vectored(bufs)
    }
    /// # System calls
    /// None performed.
    fn flush(&mut self) -> io::Result<()> {
        // You cannot flush a socket
        Ok(())
    }
}

impl Debug for UdStream {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_tuple("UdStream").field(&self.as_raw_fd()).finish()
    }
}

impl TryClone for UdStream {
    fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Self)
    }
}

impl AsFd for UdStream {
    #[inline]
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0 .0.as_fd()
    }
}
impl From<UdStream> for OwnedFd {
    #[inline]
    fn from(x: UdStream) -> Self {
        x.0 .0
    }
}
impl From<OwnedFd> for UdStream {
    #[inline]
    fn from(fd: OwnedFd) -> Self {
        UdStream(FdOps(fd))
    }
}

derive_raw!(unix: UdStream);
