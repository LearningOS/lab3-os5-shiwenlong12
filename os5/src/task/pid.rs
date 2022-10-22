// 任务pid实现。
// 将PID分配给此处的进程。同时，应用程序内核堆栈的位置根据PID确定。

use crate::config::{KERNEL_STACK_SIZE, PAGE_SIZE, TRAMPOLINE};
use crate::mm::{MapPermission, VirtAddr, KERNEL_SPACE};
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use lazy_static::*;

//实现一个同样使用简单栈式分配策略的进程标识符分配器 PidAllocator ，并将其全局实例化为 PID_ALLOCATOR
/// Process identifier allocator using stack allocation
struct PidAllocator {
    /// A new PID to be assigned
    current: usize,
    /// Recycled PID sequence
    recycled: Vec<usize>,
}

impl PidAllocator {
    pub fn new() -> Self {
        PidAllocator {
            current: 0,
            recycled: Vec::new(),
        }
    }
    pub fn alloc(&mut self) -> PidHandle {
        if let Some(pid) = self.recycled.pop() {
            PidHandle(pid)
        } else {
            self.current += 1;
            PidHandle(self.current - 1)
        }
    }
    pub fn dealloc(&mut self, pid: usize) {
        assert!(pid < self.current);
        assert!(
            !self.recycled.iter().any(|ppid| *ppid == pid),
            "pid {} has been deallocated!",
            pid
        );
        self.recycled.push(pid);
    }
}

lazy_static! {
    /// Pid allocator instance through lazy_static!
    static ref PID_ALLOCATOR: UPSafeCell<PidAllocator> =
        unsafe { UPSafeCell::new(PidAllocator::new()) };
}

/// Abstract structure of PID
//同一时间存在的所有进程都有一个自己的进程标识符，它们是互不相同的整数。
//这里将其抽象为一个 PidHandle 类型，当它的生命周期结束后，对应的整数会被编译器自动回收：
pub struct PidHandle(pub usize);

// PidAllocator::alloc 将会分配出去一个将 usize 包装之后的 PidHandle 。
// 我们将其包装为一个全局分配进程标识符的接口 pid_alloc
pub fn pid_alloc() -> PidHandle {
    PID_ALLOCATOR.exclusive_access().alloc()
}

//同时我们也需要为 PidHandle 实现 Drop Trait 来允许编译器进行自动的资源回收
impl Drop for PidHandle {
    fn drop(&mut self) {
        //println!("drop pid {}", self.0);
        PID_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}


/// KernelStack corresponding to PID
//我们将应用编号替换为进程标识符来决定每个进程内核栈在地址空间中的位置。
//在内核栈 KernelStack 中保存着它所属进程的 PID
pub struct KernelStack {
    pid: usize,
}

/// Return (bottom, top) of a kernel stack in kernel space.
//根据进程标识符计算内核栈在内核地址空间中的位置
pub fn kernel_stack_position(app_id: usize) -> (usize, usize) {
    let top = TRAMPOLINE - app_id * (KERNEL_STACK_SIZE + PAGE_SIZE);
    let bottom = top - KERNEL_STACK_SIZE;
    (bottom, top)
}

impl KernelStack {
    //new 方法可以从一个 PidHandle ，也就是一个已分配的进程标识符中对应生成一个内核栈 KernelStack
    pub fn new(pid_handle: &PidHandle) -> Self {
        let pid = pid_handle.0;
        let (kernel_stack_bottom, kernel_stack_top) = kernel_stack_position(pid);
        //将一个逻辑段插入内核地址空间 KERNEL_SPACE 中
        KERNEL_SPACE.exclusive_access().insert_framed_area(
            kernel_stack_bottom.into(),
            kernel_stack_top.into(),
            MapPermission::R | MapPermission::W,
        );
        KernelStack { pid: pid_handle.0 }
    }
    #[allow(unused)]
    /// Push a variable of type T into the top of the KernelStack and return its raw pointer
    //将一个类型为 T 的变量压入内核栈顶并返回其裸指针， 这也是一个泛型函数。
    pub fn push_on_top<T>(&self, value: T) -> *mut T
    where
        T: Sized,
    {
        let kernel_stack_top = self.get_top();
        let ptr_mut = (kernel_stack_top - core::mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr_mut = value;
        }
        ptr_mut
    }
    //获取当前内核栈顶在内核地址空间中的地址。
    pub fn get_top(&self) -> usize {
        let (_, kernel_stack_top) = kernel_stack_position(self.pid);
        kernel_stack_top
    }
}

//内核栈 KernelStack 用到了 RAII 的思想，具体来说，实际保存它的物理页帧的生命周期被绑定到它下面，
//当 KernelStack 生命周期结束后，这些物理页帧也将会被编译器自动回收
//为 KernelStack 实现 Drop Trait，一旦它的生命周期结束，就将内核地址空间中对应的逻辑段删除，
//为此在 MemorySet 中新增了一个名为 remove_area_with_start_vpn 的方法
impl Drop for KernelStack {
    fn drop(&mut self) {
        let (kernel_stack_bottom, _) = kernel_stack_position(self.pid);
        let kernel_stack_bottom_va: VirtAddr = kernel_stack_bottom.into();
        KERNEL_SPACE
            .exclusive_access()
            .remove_area_with_start_vpn(kernel_stack_bottom_va.into());
    }
}