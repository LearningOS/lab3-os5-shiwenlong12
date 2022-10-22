//! Types related to task management & Functions for completely changing TCB

use super::TaskContext;
use super::{pid_alloc, KernelStack, PidHandle};
use crate::config::{TRAP_CONTEXT, MAX_SYSCALL_NUM};
use crate::mm::{MemorySet, PhysPageNum, VirtAddr, KERNEL_SPACE};
use crate::sync::UPSafeCell;
use crate::trap::{trap_handler, TrapContext};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::cell::RefMut;

/// Task control block structure
/// Directly save the contents that will not change during running
// 直接保存运行中不会更改的内容
//在初始化之后就不再变化的作为一个字段直接放在任务控制块中。
pub struct TaskControlBlock {
    // immutable
    /// 进程标识符
    pub pid: PidHandle,
    /// Kernel stack corresponding to PID
    //PID对应的内核栈
    pub kernel_stack: KernelStack,
    // mutable
    inner: UPSafeCell<TaskControlBlockInner>,
}

///包含更多流程内容的结构
///存储将在操作期间更改的内容，并由UPSafeCell包装以提供互斥
//注意我们在维护父子进程关系的时候大量用到了智能指针 Arc/Weak ，
//当且仅当它的引用计数变为 0 的时候，进程控制块以及被绑定到它上面的各类资源才会被回收。
pub struct TaskControlBlockInner {
    //指出了应用地址空间中的 Trap 上下文被放在的物理页帧的物理页号。
    pub trap_cx_ppn: PhysPageNum,
    //应用数据仅有可能出现在应用地址空间低于 base_size 字节的区域中。
    //借助它我们可以清楚的知道应用有多少数据驻留在内存中。
    pub base_size: usize,
    /// 保存任务上下文，用于任务切换。
    pub task_cx: TaskContext,
    /// 维护当前进程的执行状态。
    pub task_status: TaskStatus,
    ///  表示应用地址空间。
    pub memory_set: MemorySet,
    /// 指向当前进程的父进程（如果存在的话）。
    /// 注意我们使用 Weak 而非 Arc 来包裹另一个任务控制块，因此这个智能指针将不会影响父进程的引用计数。
    pub parent: Option<Weak<TaskControlBlock>>,
    /// 将当前进程的所有子进程的任务控制块以 Arc 智能指针的形式保存在一个向量中，这样才能够更方便的找到它们。
    pub children: Vec<Arc<TaskControlBlock>>,
    //当进程调用 exit 系统调用主动退出或者执行出错由内核终止的时候，
    //它的退出码 exit_code 会被内核保存在它的任务控制块中，
    //并等待它的父进程通过 waitpid 回收它的资源的同时也收集它的 PID 以及退出码。
    pub exit_code: i32,

    pub start_time: usize,
    pub syscall_times: [u32; MAX_SYSCALL_NUM],

    pub priority: u8,
    pub pass: usize,
}

/// Simple access to its internal fields
//提供的方法主要是对于它内部字段的快捷访问
impl TaskControlBlockInner {
    /*
    pub fn get_task_cx_ptr2(&self) -> *const usize {
        &self.task_cx_ptr as *const usize
    }
    */
    pub fn get_trap_cx(&self) -> &'static mut TrapContext {
        self.trap_cx_ppn.get_mut()
    }
    pub fn get_user_token(&self) -> usize {
        self.memory_set.token()
    }
    fn get_status(&self) -> TaskStatus {
        self.task_status
    }
    pub fn is_zombie(&self) -> bool {
        self.get_status() == TaskStatus::Zombie
    }
}

impl TaskControlBlock {
    //尝试获取互斥锁来得到 TaskControlBlockInner 的可变引用。
    pub fn inner_exclusive_access(&self) -> RefMut<'_, TaskControlBlockInner> {
        self.inner.exclusive_access()
    }

    //new 用来创建一个新的进程，目前仅用于内核中手动创建唯一一个初始进程 initproc 。
    pub fn new(elf_data: &[u8]) -> Self {
        // 解析 ELF 得到应用地址空间 memory_set ，
        //用户栈在应用地址空间中的位置 user_sp 以及应用的入口点 entry_point 。
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        //手动查页表找到应用地址空间中的 Trap 上下文实际所在的物理页帧。
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).into())
            .unwrap()
            .ppn();
        //在内核空间中分配进程标识符和内核栈,并记录下内核栈在内核地址空间的位置 kernel_stack_top 。
        let pid_handle = pid_alloc();
        let kernel_stack = KernelStack::new(&pid_handle);
        let kernel_stack_top = kernel_stack.get_top();
        // push a task context which goes to trap_return to the top of kernel stack
        //整合之前的部分信息创建进程控制块 task_control_block 。
        let task_control_block = Self {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: user_sp,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: None,
                    children: Vec::new(),
                    exit_code: 0,
                    priority: 16,
                    pass: 0,

                    start_time: 0,
                    syscall_times: [0; MAX_SYSCALL_NUM],
                })
            },
        };
        // prepare TrapContext in user space
        //初始化位于该进程应用地址空间中的 Trap 上下文，使得第一次进入用户态时，
        //能正确跳转到应用入口点并设置好用户栈， 同时也保证在 Trap 的时候用户态能正确进入内核态。
        let trap_cx = task_control_block.inner_exclusive_access().get_trap_cx();
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            kernel_stack_top,
            trap_handler as usize,
        );
        task_control_block
    }
    /// exec 用来实现 exec 系统调用，即当前进程加载并执行另一个 ELF 格式可执行文件。
    pub fn exec(&self, elf_data: &[u8]) {
        // memory_set with elf program headers/trampoline/trap context/user stack
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).into())
            .unwrap()
            .ppn();

        // **** access inner exclusively
        let mut inner = self.inner_exclusive_access();
        // substitute memory_set
        //从 ELF 生成一个全新的地址空间并直接替换进来，
        //原有地址空间生命周期结束，里面包含的全部物理页帧都会被回收
        inner.memory_set = memory_set;
        // update trap_cx ppn
        //修改新的地址空间中的 Trap 上下文，
        inner.trap_cx_ppn = trap_cx_ppn;
        // initialize trap_cx
        //将解析得到的应用入口点、用户栈位置以及一些内核的信息进行初始化，这样才能正常实现 Trap 机制。
        let trap_cx = inner.get_trap_cx();
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            self.kernel_stack.get_top(),
            trap_handler as usize,
        );
        // **** release inner automatically
    }
    ///fork 用来实现 fork 系统调用，即当前进程 fork 出来一个与之几乎相同的子进程。
    //从父进程的进程控制块创建一份子进程的控制块
    pub fn fork(self: &Arc<TaskControlBlock>) -> Arc<TaskControlBlock> {
        // ---- access parent PCB exclusively
        let mut parent_inner = self.inner_exclusive_access();
        // copy user space(include trap context)
        //子进程的地址空间不是通过解析 ELF，
        //而是通过调用 MemorySet::from_existed_user 复制父进程地址空间得到的
        let memory_set = MemorySet::from_existed_user(&parent_inner.memory_set);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).into())
            .unwrap()
            .ppn();
        // alloc a pid and a kernel stack in kernel space
        //在内核空间中分配pid和内核栈
        let pid_handle = pid_alloc();
        let kernel_stack = KernelStack::new(&pid_handle);
        let kernel_stack_top = kernel_stack.get_top();
        let task_control_block = Arc::new(TaskControlBlock {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    priority: 16,
                    pass: 0,

                    start_time: 0,
                    syscall_times: [0; MAX_SYSCALL_NUM],
                })
            },
        });
        // add child
        //将子进程插入到父进程的孩子向量 children 中
        parent_inner.children.push(task_control_block.clone());
        // modify kernel_sp in trap_cx
        // **** access children PCB exclusively
        let trap_cx = task_control_block.inner_exclusive_access().get_trap_cx();
        trap_cx.kernel_sp = kernel_stack_top;
        // return
        task_control_block
        // ---- release parent PCB automatically
        // **** release children PCB automatically
    }
    //以 usize 的形式返回当前进程的进程标识符。
    pub fn getpid(&self) -> usize {
        self.pid.0
    }

    //功能：新建子进程，使其执行目标程序
    //返回值：成功返回子进程id，否则返回-1。
    pub fn spawn(self: &Arc<TaskControlBlock>, _elf_data: &[u8]) -> Arc<TaskControlBlock> {
        // ---- access parent PCB exclusively
        let mut parent_inner = self.inner_exclusive_access();
        // copy user space(include trap context)
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(_elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT).into())
            .unwrap()
            .ppn();
        // alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        let kernel_stack = KernelStack::new(&pid_handle);
        let kernel_stack_top = kernel_stack.get_top();
        let task_control_block = Arc::new(TaskControlBlock {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: Some(Arc::downgrade(self)),
                    children: Vec::new(),
                    exit_code: 0,
                    priority: 16,
                    pass: 0,

                    start_time: 0,
                    syscall_times: [0; MAX_SYSCALL_NUM],
                })
            },
        });
        // add child
        parent_inner.children.push(task_control_block.clone());
        // modify kernel_sp in trap_cx
        // **** access children PCB exclusively
        let trap_cx = task_control_block.inner_exclusive_access().get_trap_cx();
        *trap_cx = TrapContext::app_init_context(entry_point, user_sp, KERNEL_SPACE.exclusive_access().token(), kernel_stack_top, trap_handler as usize);
        trap_cx.kernel_sp = kernel_stack_top;
        // return
        task_control_block
        // ---- release parent PCB automatically
        // **** release children PCB automatically
    }
}

#[derive(Copy, Clone, PartialEq)]
/// task status: UnInit, Ready, Running, Exited
pub enum TaskStatus {
    UnInit,
    Ready,
    Running,
    Zombie,
}
