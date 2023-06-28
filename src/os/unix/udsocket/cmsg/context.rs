//! Context collection for control messages.
//!
//! # The problem
//! FreeBSD has two control message types, `struct cmsgcred` and `struct sockcred`, under the same control message ID
//! `SCM_CREDS`. The only real way to tell which one of the two the message actually is is to query the `LOCAL_CREDS`
//! socket option.
//!
//! Of particular note is the fact that it is decidedly impossible to perform this check in a manner which isn't racy –
//! which is to say, calling `setsockopt` on one thread can cause another thread's readout of `getsockopt` to be
//! outdated by the time `recvmsg` is called, allowing the two control message types to be mixed up without the use of
//! unsafe Rust.
//!
//! # The solution
//! The [`Collector`] trait provides a generic interface for structs interested in collecting context from
//! ancillary-enabled I/O calls on Ud-sockets. Various other utilities in this module allow for composition of
//! collectors, if such a need ever arises.

use crate::os::unix::unixprelude::*;

/// A context collector to hook into a Ud-socket read/write operation.
#[allow(unused_variables)]
pub trait Collector {
    /// Called right before the call to `recvmsg` or `sendmsg`, providing a borrow of the file descriptor of the socket.
    fn pre_op_collect(&mut self, socket: BorrowedFd<'_>) {}
    /// Same as `pre_op_collect`, but called right after the system call with the contents of the `msghdr`'s `msg_flags`
    /// field which it will be performed with..
    fn post_op_collect(&mut self, socket: BorrowedFd<'_>, msghdr_flags: c_int) {}
}
impl<T: Collector> Collector for &mut T {
    #[inline]
    fn pre_op_collect(&mut self, socket: BorrowedFd<'_>) {
        (*self).pre_op_collect(socket);
    }
    #[inline]
    fn post_op_collect(&mut self, socket: BorrowedFd<'_>, msghdr_flags: c_int) {
        (*self).post_op_collect(socket, msghdr_flags);
    }
}
impl<T: Collector> Collector for Box<T> {
    #[inline]
    fn pre_op_collect(&mut self, socket: BorrowedFd<'_>) {
        self.as_mut().pre_op_collect(socket);
    }
    #[inline]
    fn post_op_collect(&mut self, socket: BorrowedFd<'_>, msghdr_flags: c_int) {
        self.as_mut().post_op_collect(socket, msghdr_flags);
    }
}

/// A [`Collector`] that does nothing.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Hash)]
pub struct DummyCollector;
impl Collector for DummyCollector {}
pub(super) const DUMMY_COLLECTOR: DummyCollector = DummyCollector;

/// A [`Collector`] that diverts to given closures.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Hash)]
pub struct FnCollector<F1, F2>(F1, F2);
impl<F1: FnMut(BorrowedFd<'_>), F2: FnMut(BorrowedFd<'_>, c_int)> FnCollector<F1, F2> {
    /// Creates a collector from the given two closures.
    #[inline]
    pub fn before_and_after(before: F1, after: F2) -> Self {
        Self(before, after)
    }
}
impl<F1: FnMut(BorrowedFd<'_>)> FnCollector<F1, fn(BorrowedFd<'_>, c_int)> {
    /// Creates a collector that only hooks before the call.
    #[inline]
    pub fn before(before: F1) -> Self {
        Self(before, |_, _| {})
    }
}
impl<F2: FnMut(BorrowedFd<'_>, c_int)> FnCollector<fn(BorrowedFd<'_>), F2> {
    /// Creates a collector that only hooks after the call.
    #[inline]
    pub fn after(after: F2) -> Self {
        Self(|_| {}, after)
    }
}
impl<F1: FnMut(BorrowedFd<'_>), F2: FnMut(BorrowedFd<'_>, c_int)> Collector for FnCollector<F1, F2> {
    fn pre_op_collect(&mut self, socket: BorrowedFd<'_>) {
        self.0(socket)
    }
    fn post_op_collect(&mut self, socket: BorrowedFd<'_>, msghdr_flags: c_int) {
        self.1(socket, msghdr_flags)
    }
}

/// A [`Collector`] that calls every collector in a given collection.
///
/// The collection can be any type `C` such that `&mut C` implements [`IntoIterator`] over an item time which implements
/// `Collector`.
pub struct IterCollector<C>(C);
impl<C> IterCollector<C>
where
    for<'a> &'a mut C: IntoIterator,
    for<'a> <&'a mut C as IntoIterator>::Item: Collector,
{
    /// Creates a collector that iterates over a collection of collectors.
    pub fn new(collection: C) -> Self {
        Self(collection)
    }
}

impl<C> Collector for IterCollector<C>
where
    for<'a> &'a mut C: IntoIterator,
    for<'a> <&'a mut C as IntoIterator>::Item: Collector,
{
    fn pre_op_collect(&mut self, socket: BorrowedFd<'_>) {
        for mut c in &mut self.0 {
            c.pre_op_collect(socket);
        }
    }
    fn post_op_collect(&mut self, socket: BorrowedFd<'_>, msghdr_flags: c_int) {
        for mut c in &mut self.0 {
            c.post_op_collect(socket, msghdr_flags);
        }
    }
}