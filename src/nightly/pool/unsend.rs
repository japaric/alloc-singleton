//! Fixed size memory pool with automatic deallocation of handles

use core::{
    cell::{Cell, UnsafeCell},
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ops, ptr,
};

use generic_array::{
    typenum::{consts::U256, IsLess, True},
    ArrayLength, GenericArray,
};
use owned_singleton::Singleton;
use stable_deref_trait::StableDeref;

/// A value allocated on the memory pool `P`
///
/// - `Box` never implements the `Send` or `Sync` traits.
/// - `Box` destructor returns the memory to the pool `P`
/// - `sizeof(Box<_>)` is a single byte
pub struct Box<P>
where
    P: Singleton,
    P::Type: sealed::Dealloc,
{
    _not_send_or_sync: PhantomData<*const ()>,
    _pool: PhantomData<P>,
    index: u8,
}

impl<P> Drop for Box<P>
where
    P: Singleton,
    P::Type: sealed::Dealloc,
{
    fn drop(&mut self) {
        use self::sealed::Dealloc;

        unsafe { (*P::get()).dealloc(self.index) }
    }
}

impl<T, N, P> ops::Deref for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
{
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*((*P::get()).memory.get() as *const T).add(usize::from(self.index)) }
    }
}

impl<T, N, P> ops::DerefMut for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
{
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *((*P::get()).memory.get() as *mut T).add(usize::from(self.index)) }
    }
}

impl<T, N, P> Box<P>
where
    P: Singleton<Type = Pool<T, N>> + ops::Deref<Target = Pool<T, N>>,
    N: ArrayLength<T>,
{
    /// Allocates the given `value` on the pool
    ///
    /// # Errors
    ///
    /// If the memory pool has been exhausted an error containing `value` is returned
    pub fn new(pool: &P, value: T) -> Result<Box<P>, T> {
        unsafe {
            assert!(mem::size_of::<T>() > 0);

            if pool.initialized.get() < N::U8 {
                let index = pool.initialized.get();

                let p = (pool.memory.get() as *mut T).add(usize::from(index));

                *(p as *mut u8) = index + 1;
                pool.initialized.set(index + 1);
            }

            if pool.free.get() != 0 {
                let index = pool.head.get();
                let p = (pool.memory.get() as *mut T).add(usize::from(index));
                pool.head.set(*(p as *const u8));

                pool.free.set(pool.free.get() - 1);

                ptr::write(p, value);

                Ok(Box {
                    _not_send_or_sync: PhantomData,
                    _pool: PhantomData,
                    index,
                })
            } else {
                Err(value)
            }
        }
    }
}

unsafe impl<T, N, P> StableDeref for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
{
}
/// A fixed-size memory pool that can NOT be sent across threads
///
/// # Example
///
/// ```
/// use owned_singleton::Singleton;
/// use alloc_singleton::nightly::{consts::*, pool::unsend::{Box, Pool}};
///
/// #[Singleton]
/// static P: Pool<[u8; 128], U2> = Pool::new();
///
/// let pool = unsafe { P::new() };
///
/// let mut buffer: Box<P> = Box::new(&pool, [0; 128]).ok().unwrap();
///
/// //  ..
///
/// // return the memory to the pool
/// drop(buffer);
/// ```
pub struct Pool<T, N>
where
    N: ArrayLength<T>,
{
    _not_send_or_sync: PhantomData<*const ()>,
    free: Cell<u8>,
    head: Cell<u8>,
    initialized: Cell<u8>,
    memory: UnsafeCell<MaybeUninit<GenericArray<T, N>>>,
}

unsafe impl<T, N> sealed::Dealloc for Pool<T, N>
where
    N: ArrayLength<T>,
{
    unsafe fn dealloc(&self, index: u8) {
        let p = (self.memory.get() as *mut T).add(usize::from(index));

        ptr::drop_in_place(p);

        *(p as *mut u8) = self.head.get();

        self.free.set(self.free.get() + 1);
        self.head.set(index);
    }
}

impl<T, N> Pool<T, N>
where
    N: ArrayLength<T> + IsLess<U256, Output = True>,
{
    /// Creates a new memory pool
    pub const fn new() -> Self {
        Pool {
            _not_send_or_sync: PhantomData,
            free: Cell::new(N::U8),
            head: Cell::new(0),
            initialized: Cell::new(0),
            memory: UnsafeCell::new(MaybeUninit::uninitialized()),
        }
    }
}

mod sealed {
    pub unsafe trait Dealloc {
        unsafe fn dealloc(&self, value: u8);
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use generic_array::typenum::consts::*;
    use owned_singleton::Singleton;

    use super::{Box, Pool};

    #[test]
    fn sanity() {
        #[Singleton]
        static mut P: Pool<i8, U4> = Pool::new();

        let ref pool = unsafe { P::new() };

        let _0 = Box::new(pool, -1).unwrap();
        assert_eq!(*_0, -1);
        assert_eq!(_0.index, 0);
        assert_eq!(pool.head.get(), 1);
        assert_eq!(pool.free.get(), 3);
        assert_eq!(pool.initialized.get(), 1);

        let _1 = Box::new(pool, -2).unwrap();
        assert_eq!(*_1, -2);
        assert_eq!(_1.index, 1);
        assert_eq!(pool.head.get(), 2);
        assert_eq!(pool.free.get(), 2);
        assert_eq!(pool.initialized.get(), 2);

        let _2 = Box::new(pool, -3).unwrap();
        assert_eq!(*_2, -3);
        assert_eq!(_2.index, 2);
        assert_eq!(pool.head.get(), 3);
        assert_eq!(pool.free.get(), 1);
        assert_eq!(pool.initialized.get(), 3);

        drop(_0);

        assert_eq!(pool.head.get(), 0);
        assert_eq!(pool.free.get(), 2);
        assert_eq!(pool.initialized.get(), 3);
        assert_eq!(unsafe { *(pool.memory.get() as *const i8) }, 3);

        drop(_2);
        assert_eq!(pool.head.get(), 2);
        assert_eq!(pool.free.get(), 3);
        assert_eq!(pool.initialized.get(), 3);
        assert_eq!(unsafe { *((pool.memory.get() as *const i8).add(2)) }, 0);

        let _2 = Box::new(pool, -4).unwrap();
        assert_eq!(*_2, -4);
        assert_eq!(_2.index, 2);
        assert_eq!(pool.head.get(), 0);
        assert_eq!(pool.free.get(), 2);
        assert_eq!(pool.initialized.get(), 4);
        assert_eq!(unsafe { *((pool.memory.get() as *const i8).add(3)) }, 4);
    }

    #[test]
    fn destructor() {
        static COUNT: AtomicUsize = AtomicUsize::new(0);

        pub struct A(usize);

        impl A {
            fn new() -> Self {
                A(COUNT.fetch_add(1, Ordering::SeqCst))
            }
        }

        impl Drop for A {
            fn drop(&mut self) {
                COUNT.fetch_sub(1, Ordering::SeqCst);
            }
        }

        #[Singleton]
        static mut P: Pool<A, U4> = Pool::new();

        let pool = unsafe { P::new() };

        let _0 = Box::new(&pool, A::new()).ok().unwrap();
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);

        let _1 = Box::new(&pool, A::new()).ok().unwrap();
        assert_eq!(COUNT.load(Ordering::SeqCst), 2);

        // Dropping the `Box` should run `A`'s destructor
        drop(_0);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);

        // Dropping the handle to the `Pool` should not run any destructor
        drop(pool);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);

        // `Box`es can outlive the handle to their `Pool` (since the `Pool` is statically allocated)
        drop(_1);
        assert_eq!(COUNT.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn empty() {
        #[Singleton]
        static mut P: Pool<i8, U4> = Pool::new();

        let ref pool = unsafe { P::new() };

        let _0 = Box::new(pool, -1).unwrap();
        let _1 = Box::new(pool, -1).unwrap();
        let _2 = Box::new(pool, -1).unwrap();
        let _3 = Box::new(pool, -1).unwrap();

        assert!(Box::new(pool, -1).is_err());

        drop(_0);
        drop(_2);

        let _2 = Box::new(pool, -1).unwrap();
        assert_eq!(_2.index, 2);

        let _0 = Box::new(pool, -1).unwrap();
        assert_eq!(_0.index, 0);
    }

    #[test]
    fn max_capacity() {
        #[Singleton]
        static mut P: Pool<i8, U255> = Pool::new();

        let ref pool = unsafe { P::new() };

        let mut xs = vec![];
        for _ in 0..255 {
            xs.push(Box::new(pool, -1).unwrap());
        }

        assert!(Box::new(pool, -1).is_err())
    }
}
