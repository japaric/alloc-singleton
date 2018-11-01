//! Fixed size memory pool

use core::{fmt, marker::PhantomData, mem, ops, ptr, u8};

use as_slice::{AsMutSlice, AsSlice};
use owned_singleton::Singleton;
use stable_deref_trait::StableDeref;

/// A value allocated on the memory pool `Pool<M>`
///
/// - `sizeof(Box<_>)` is a single byte
/// - `Box<M>` implements `Send` if it derefs to a type `T` that implements `Send`
/// - `Box<M>` implements `Sync` if it derefs to a type `T` that implements `Sync`
pub struct Box<M>
where
    M: Singleton,
{
    _memory: PhantomData<M>,
    _not_send_or_sync: PhantomData<*const ()>,
    index: u8,
}

impl<T, M> ops::Deref for Box<M>
where
    M: Singleton,
    M::Type: AsSlice<Element = T>,
{
    type Target = T;

    fn deref(&self) -> &T {
        unsafe {
            (*M::get())
                .as_slice()
                .get_unchecked(usize::from(self.index))
        }
    }
}

impl<T, M> ops::DerefMut for Box<M>
where
    M: Singleton,
    M::Type: AsMutSlice<Element = T>,
{
    fn deref_mut(&mut self) -> &mut T {
        unsafe {
            (*M::get())
                .as_mut_slice()
                .get_unchecked_mut(usize::from(self.index))
        }
    }
}

unsafe impl<T, M> StableDeref for Box<M>
where
    M: Singleton,
    M::Type: AsMutSlice<Element = T>,
{
}

impl<T, M> fmt::Debug for Box<M>
where
    M: Singleton,
    M::Type: AsSlice<Element = T>,
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        T::fmt(&**self, f)
    }
}

impl<T, M> fmt::Display for Box<M>
where
    M: Singleton,
    M::Type: AsSlice<Element = T>,
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        T::fmt(&**self, f)
    }
}

unsafe impl<T, M> Send for Box<M>
where
    M: Singleton,
    M::Type: AsSlice<Element = T>,
    T: Send,
{
}

unsafe impl<T, M> Sync for Box<M>
where
    M: Singleton,
    M::Type: AsSlice<Element = T>,
    T: Sync,
{
}

/// A fixed-size memory pool backed by the memory chunk behind the owned singleton `M`
///
/// # Example
///
/// ```
/// use alloc_singleton::stable::pool::{Box, Pool};
/// use owned_singleton::Singleton;
///
/// #[Singleton]
/// static mut M: [[u8; 128]; 4] = [[0; 128]; 4];
///
/// let mut pool = Pool::new(unsafe { M::new() });
///
/// let buffer: Box<M> = pool.alloc([0; 128]).ok().unwrap();
///
/// // ..
///
/// // return the memory to the pool or the memory will be leaked
/// pool.dealloc(buffer);
/// ```
pub struct Pool<M>
where
    M: Singleton,
{
    free: u8,
    head: u8,
    initialized: u8,
    memory: M,
}

impl<T, A, M> Pool<M>
where
    M: Singleton<Type = A> + ops::DerefMut<Target = A>,
    A: AsMutSlice<Element = T>,
{
    /// Creates a memory pool that allocates on the given `memory` chunk
    ///
    /// The resulting `Pool` is semantically a singleton: there can only exist a single instead of
    /// `Pool<#M>` for any concrete `#M`
    ///
    /// *NOTE*: `Pool` will have a maximum capacity of 25**5** elements, even if `M::Type` has a
    /// bigger capacity.
    ///
    /// # Panics
    ///
    /// This constructor panics if `sizeof(M::Type::Element)` is a zero. In other words, `Pool`
    /// doesn't support ZST.
    #[allow(unused_variables)]
    pub fn new(memory: M) -> Self {
        assert!(mem::size_of::<T>() > 0);

        let capacity = memory.as_slice().len();

        Pool {
            free: if capacity > usize::from(u8::MAX) {
                u8::MAX
            } else {
                capacity as u8
            },
            head: 0,
            initialized: 0,
            memory,
        }
    }

    /// Allocates the given `value` on the memory pool
    ///
    /// # Errors
    ///
    /// If the memory pool has been exhausted an error containing `value` is returned
    pub fn alloc(&mut self, value: T) -> Result<Box<M>, T> {
        unsafe {
            let n = self.memory.as_slice().len() as u8;

            if self.initialized < n {
                let index = self.initialized;

                let p: *mut T = self
                    .memory
                    .as_mut_slice()
                    .get_unchecked_mut(usize::from(index));

                // the memory (`M`) starts initialized; we have to deinitialize it before we
                // overwrite its contents
                ptr::drop_in_place(p);

                *(p as *mut u8) = index + 1;
                self.initialized += 1;
            }

            if self.free != 0 {
                let index = self.head;
                let p = self
                    .memory
                    .as_mut_slice()
                    .as_mut_ptr()
                    .add(usize::from(index));
                self.head = *(p as *const u8);

                self.free -= 1;

                ptr::write(p, value);

                Ok(Box {
                    _memory: PhantomData,
                    _not_send_or_sync: PhantomData,
                    index,
                })
            } else {
                Err(value)
            }
        }
    }

    /// Deallocates the given `value` and returns the memory to the pool
    ///
    /// *NOTE*: `M::Type::Element`'s destructor (if any) will run on `value`
    pub fn dealloc(&mut self, value: Box<M>) {
        unsafe {
            let p: *mut T = self
                .memory
                .as_mut_slice()
                .get_unchecked_mut(value.index as usize);

            ptr::drop_in_place(p);

            *(p as *mut u8) = self.head;

            self.free += 1;
            self.head = value.index;
        }
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use owned_singleton::Singleton;

    use super::Pool;

    #[test]
    fn sanity() {
        #[Singleton]
        static mut M: [i8; 4] = [0; 4];

        let mut pool = Pool::new(unsafe { M::new() });

        let _0 = pool.alloc(-1).unwrap();
        assert_eq!(*_0, -1);
        assert_eq!(_0.index, 0);
        assert_eq!(pool.head, 1);
        assert_eq!(pool.free, 3);
        assert_eq!(pool.initialized, 1);

        let _1 = pool.alloc(-2).unwrap();
        assert_eq!(*_1, -2);
        assert_eq!(_1.index, 1);
        assert_eq!(pool.head, 2);
        assert_eq!(pool.free, 2);
        assert_eq!(pool.initialized, 2);

        let _2 = pool.alloc(-3).unwrap();
        assert_eq!(*_2, -3);
        assert_eq!(_2.index, 2);
        assert_eq!(pool.head, 3);
        assert_eq!(pool.free, 1);
        assert_eq!(pool.initialized, 3);

        pool.dealloc(_0);
        assert_eq!(pool.head, 0);
        assert_eq!(pool.free, 2);
        assert_eq!(pool.initialized, 3);
        assert_eq!(unsafe { (*M::get())[0] }, 3);

        pool.dealloc(_2);
        assert_eq!(pool.head, 2);
        assert_eq!(pool.free, 3);
        assert_eq!(pool.initialized, 3);
        assert_eq!(unsafe { (*M::get())[2] }, 0);

        let _2 = pool.alloc(-4).unwrap();
        assert_eq!(*_2, -4);
        assert_eq!(_2.index, 2);
        assert_eq!(pool.head, 0);
        assert_eq!(pool.free, 2);
        assert_eq!(pool.initialized, 4);
        assert_eq!(unsafe { (*M::get())[3] }, 4);
    }

    // test that deallocated values are dropped
    #[test]
    fn destructor() {
        static COUNT: AtomicUsize = AtomicUsize::new(1);

        pub struct A(u32);

        impl A {
            fn new() -> Self {
                COUNT.fetch_add(1, Ordering::SeqCst);
                A(0)
            }
        }

        impl Drop for A {
            fn drop(&mut self) {
                COUNT.fetch_sub(1, Ordering::SeqCst);
            }
        }

        #[Singleton]
        static mut M: [A; 4] = [A(0), A(1), A(2), A(3)];

        {
            let mut pool = Pool::new(unsafe { M::new() });

            let _0 = pool.alloc(A::new()).ok().unwrap();
            assert_eq!(COUNT.load(Ordering::SeqCst), 1);

            // Deallocating the `Box` should run `A`'s destructor
            pool.dealloc(_0);
            assert_eq!(COUNT.load(Ordering::SeqCst), 0);
        }

        assert_eq!(COUNT.load(Ordering::SeqCst), 0);
    }

    // test that not explicitly deallocated values are leaked
    #[test]
    fn leak() {
        static COUNT: AtomicUsize = AtomicUsize::new(1);

        pub struct A(u32);

        impl A {
            fn new() -> Self {
                COUNT.fetch_add(1, Ordering::SeqCst);
                A(0)
            }
        }

        impl Drop for A {
            fn drop(&mut self) {
                COUNT.fetch_sub(1, Ordering::SeqCst);
            }
        }

        #[Singleton]
        static mut M: [A; 4] = [A(0), A(1), A(2), A(3)];

        {
            let mut pool = Pool::new(unsafe { M::new() });

            let _0 = pool.alloc(A::new()).ok().unwrap();
            assert_eq!(COUNT.load(Ordering::SeqCst), 1);

            // destroying the object does NOT invoke `A`'s destructor
            drop(_0);

            assert_eq!(COUNT.load(Ordering::SeqCst), 1);

            let _1 = pool.alloc(A::new()).ok().unwrap();
            assert_eq!(COUNT.load(Ordering::SeqCst), 1);

            drop(_1);

            assert_eq!(COUNT.load(Ordering::SeqCst), 1);

            // destroying the object pool does NOT invoke `A`'s destructor
            drop(pool);
        }

        assert_eq!(COUNT.load(Ordering::SeqCst), 1);
    }

    // test that exhausting the pool and then deallocating works correctly
    #[test]
    fn empty() {
        #[Singleton]
        static mut M: [i8; 4] = [0; 4];

        let mut pool = Pool::new(unsafe { M::new() });

        let _0 = pool.alloc(-1).unwrap();
        let _1 = pool.alloc(-1).unwrap();
        let _2 = pool.alloc(-1).unwrap();
        let _3 = pool.alloc(-1).unwrap();

        assert!(pool.alloc(-1).is_err());

        pool.dealloc(_0);
        pool.dealloc(_2);

        let _2 = pool.alloc(-1).unwrap();
        assert_eq!(_2.index, 2);

        let _0 = pool.alloc(-1).unwrap();
        assert_eq!(_0.index, 0);
    }

    #[test]
    fn max_capacity() {
        #[Singleton]
        static mut M: [i8; 256] = [0; 256];

        let mut pool = Pool::new(unsafe { M::new() });

        for _ in 0..255 {
            assert!(pool.alloc(-1).is_ok());
        }

        assert!(pool.alloc(-1).is_err());
    }
}
