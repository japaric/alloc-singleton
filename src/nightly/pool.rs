//! Fixed size memory pool

pub mod unsend;

use core::{
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
/// - `Box` must be explicitly deallocated or memory will be leaked
/// - `sizeof(Box<_>)` is a single byte
/// - `Box<P>` implements `Send` if it derefs to a type `T` that implements `Send`
/// - `Box<P>` implements `Sync` if it derefs to a type `T` that implements `Sync`
pub struct Box<P>
where
    P: Singleton,
{
    _not_send_or_sync: PhantomData<*const ()>,
    _pool: PhantomData<P>,
    index: u8,
}

impl<T, N, P> Box<P>
where
    P: Singleton<Type = Pool<T, N>> + ops::DerefMut<Target = Pool<T, N>>,
    N: ArrayLength<T>,
{
    /// Allocates the given `value` on the pool
    ///
    /// # Errors
    ///
    /// If the memory pool has been exhausted an error containing `value` is returned
    pub fn new(pool: &mut P, value: T) -> Result<Box<P>, T> {
        unsafe {
            assert!(mem::size_of::<T>() > 0);

            if pool.initialized < N::U8 {
                let index = pool.initialized;

                let p = (pool.memory.as_mut_ptr() as *mut T).add(usize::from(index));

                *(p as *mut u8) = index + 1;
                pool.initialized += 1;
            }

            if pool.free != 0 {
                let index = pool.head;
                let p = (pool.memory.as_mut_ptr() as *mut T).add(usize::from(index));
                pool.head = *(p as *const u8);

                pool.free -= 1;

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

    /// Returns this `Box` to the `pool`
    ///
    /// *NOTE*: This method must be invoked as `Box::free(x, pool)`, `x.free(pool)` doesn't compile.
    pub fn free(self, pool: &mut P) {
        unsafe {
            let p = (pool.memory.as_mut_ptr() as *mut T).add(usize::from(self.index));

            ptr::drop_in_place(p);

            *(p as *mut u8) = pool.head;

            pool.free += 1;
            pool.head = self.index;
        }
    }
}

impl<T, N, P> ops::Deref for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
{
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*((*P::get()).memory.as_ptr() as *const T).add(usize::from(self.index)) }
    }
}

impl<T, N, P> ops::DerefMut for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
{
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *((*P::get()).memory.as_mut_ptr() as *mut T).add(usize::from(self.index)) }
    }
}

unsafe impl<T, N, P> Send for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
    T: Send,
{
}

unsafe impl<T, N, P> Sync for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
    T: Sync,
{
}

unsafe impl<T, N, P> StableDeref for Box<P>
where
    P: Singleton<Type = Pool<T, N>>,
    N: ArrayLength<T>,
{
}

/// A fixed-size memory pool
///
/// # Example
///
/// ```
/// use owned_singleton::Singleton;
/// use alloc_singleton::nightly::{consts::*, pool::{Box, Pool}};
///
/// #[Singleton]
/// static mut P: Pool<[u8; 128], U4> = Pool::new();
///
/// let mut pool = unsafe { P::new() };
///
/// let mut buffer: Box<P> = Box::new(&mut pool, [0; 128]).ok().unwrap();
///
/// //  ..
///
/// // return the memory to the pool or the memory will be leaked
/// Box::free(buffer, &mut pool);
/// ```
pub struct Pool<T, N>
where
    N: ArrayLength<T>,
{
    _not_send_or_sync: PhantomData<*const ()>,
    free: u8,
    head: u8,
    initialized: u8,
    memory: MaybeUninit<GenericArray<T, N>>,
}

impl<T, N> Pool<T, N>
where
    N: ArrayLength<T> + IsLess<U256, Output = True>,
{
    /// Creates a new memory pool
    pub const fn new() -> Self {
        Pool {
            _not_send_or_sync: PhantomData,
            free: N::U8,
            head: 0,
            initialized: 0,
            memory: MaybeUninit::uninitialized(),
        }
    }
}

unsafe impl<T, N> Send for Pool<T, N>
where
    N: ArrayLength<T>,
    T: Send,
{
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

        let ref mut pool = unsafe { P::new() };

        let _0 = Box::new(pool, -1).unwrap();
        assert_eq!(*_0, -1);
        assert_eq!(_0.index, 0);
        assert_eq!(pool.head, 1);
        assert_eq!(pool.free, 3);
        assert_eq!(pool.initialized, 1);

        let _1 = Box::new(pool, -2).unwrap();
        assert_eq!(*_1, -2);
        assert_eq!(_1.index, 1);
        assert_eq!(pool.head, 2);
        assert_eq!(pool.free, 2);
        assert_eq!(pool.initialized, 2);

        let _2 = Box::new(pool, -3).unwrap();
        assert_eq!(*_2, -3);
        assert_eq!(_2.index, 2);
        assert_eq!(pool.head, 3);
        assert_eq!(pool.free, 1);
        assert_eq!(pool.initialized, 3);

        Box::free(_0, pool);
        assert_eq!(pool.head, 0);
        assert_eq!(pool.free, 2);
        assert_eq!(pool.initialized, 3);
        assert_eq!(unsafe { *(pool.memory.as_ptr() as *const i8) }, 3);

        Box::free(_2, pool);
        assert_eq!(pool.head, 2);
        assert_eq!(pool.free, 3);
        assert_eq!(pool.initialized, 3);
        assert_eq!(unsafe { *((pool.memory.as_ptr() as *const i8).add(2)) }, 0);

        let _2 = Box::new(pool, -4).unwrap();
        assert_eq!(*_2, -4);
        assert_eq!(_2.index, 2);
        assert_eq!(pool.head, 0);
        assert_eq!(pool.free, 2);
        assert_eq!(pool.initialized, 4);
        assert_eq!(unsafe { *((pool.memory.as_ptr() as *const i8).add(3)) }, 4);
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

        let mut pool = unsafe { P::new() };

        let _0 = Box::new(&mut pool, A::new()).ok().unwrap();
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);

        let _1 = Box::new(&mut pool, A::new()).ok().unwrap();
        assert_eq!(COUNT.load(Ordering::SeqCst), 2);

        // Freeing the `Box` should run `A`'s destructor
        Box::free(_0, &mut pool);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);

        // Dropping the handle to the `Pool` should not run any destructor
        drop(pool);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);

        // Dropping the `Box` should not run any destructor
        drop(_1);
        assert_eq!(COUNT.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty() {
        #[Singleton]
        static mut P: Pool<i8, U4> = Pool::new();

        let ref mut pool = unsafe { P::new() };

        let _0 = Box::new(pool, -1).unwrap();
        let _1 = Box::new(pool, -1).unwrap();
        let _2 = Box::new(pool, -1).unwrap();
        let _3 = Box::new(pool, -1).unwrap();

        assert!(Box::new(pool, -1).is_err());

        Box::free(_0, pool);
        Box::free(_2, pool);

        let _2 = Box::new(pool, -1).unwrap();
        assert_eq!(_2.index, 2);

        let _0 = Box::new(pool, -1).unwrap();
        assert_eq!(_0.index, 0);
    }

    #[test]
    fn max_capacity() {
        #[Singleton]
        static mut P: Pool<i8, U255> = Pool::new();

        let ref mut pool = unsafe { P::new() };

        let mut xs = vec![];
        for _ in 0..255 {
            xs.push(Box::new(pool, -1).unwrap());
        }

        assert!(Box::new(pool, -1).is_err())
    }
}
