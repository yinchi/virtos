//! `virtio_drivers::Hal` implementation.
//!
//! Deliberately trivial: every `virtio,mmio` node in the DTB carries a `dma-coherent` property
//! (DMA and CPU caches are always consistent), and this kernel's MMU is one flat identity
//! mapping (`arch/mmu.rs`) -- physical and virtual addresses are always equal. There is also no
//! IOMMU, so `share`/`unshare` are no-ops.

use core::ptr::NonNull;
use core::sync::atomic::{AtomicUsize, Ordering};

use virtio_drivers::{BufferDirection, Hal, PAGE_SIZE, PhysAddr};

/// Number of pages in the DMA pool: room for a block device's virtqueue.
const DMA_POOL_PAGES: usize = 64;
const DMA_POOL_SIZE: usize = DMA_POOL_PAGES * PAGE_SIZE;

#[repr(align(4096))]
struct AlignedPool([u8; DMA_POOL_SIZE]);

static mut DMA_POOL: AlignedPool = AlignedPool([0; DMA_POOL_SIZE]);

/// Byte offset of the next unused page in `DMA_POOL`. Only ever grows.
static DMA_POOL_NEXT: AtomicUsize = AtomicUsize::new(0);

pub struct VirtioHalImpl;

// SAFETY: see this module's doc comment -- coherent DMA, identity-mapped memory, no IOMMU.
unsafe impl Hal for VirtioHalImpl {
    fn dma_alloc(pages: usize, _direction: BufferDirection) -> (PhysAddr, NonNull<u8>) {
        let size = pages * PAGE_SIZE;
        let offset = DMA_POOL_NEXT.fetch_add(size, Ordering::Relaxed);

        if offset + size > DMA_POOL_SIZE {
            return (0, NonNull::dangling());
        }

        // SAFETY: `offset..offset+size` was just reserved above and isn't handed out to anyone
        // else -- DMA_POOL_NEXT only ever increases.
        let ptr = unsafe { &raw mut DMA_POOL.0[offset] };
        (ptr as usize as PhysAddr, NonNull::new(ptr).unwrap())
    }

    unsafe fn dma_dealloc(_paddr: PhysAddr, _vaddr: NonNull<u8>, _pages: usize) -> i32 {
        unimplemented!("every device lives for the program's entire lifetime; nothing ever frees")
    }

    unsafe fn mmio_phys_to_virt(paddr: PhysAddr, _size: usize) -> NonNull<u8> {
        NonNull::new(paddr as usize as *mut u8).expect("mmio_phys_to_virt: null physical address")
    }

    unsafe fn share(buffer: NonNull<[u8]>, _direction: BufferDirection) -> PhysAddr {
        buffer.as_ptr() as *mut u8 as usize as PhysAddr
    }

    unsafe fn unshare(_paddr: PhysAddr, _buffer: NonNull<[u8]>, _direction: BufferDirection) {}
}
