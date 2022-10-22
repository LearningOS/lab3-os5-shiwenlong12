// 实现[`Processor`]和控制流的交集
//将任务管理器对于 CPU 的监控职能拆分到处理器管理结构 Processor 中去
// 在这里，用户应用程序在CPU中持续运行，记录CPU的当前运行状态，并执行不同应用程序控制流的替换和转移。

use super::__switch;
use super::{fetch_task, TaskStatus};
use super::{TaskContext, TaskControlBlock};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::sync::Arc;
use lazy_static::*;

use crate::{config, mm, timer};

/// Processor management structure
//处理器管理结构 Processor 负责维护从任务管理器 TaskManager 分离出去的那部分 CPU 状态：
pub struct Processor {
    /// 表示在当前处理器上正在执行的任务
    current: Option<Arc<TaskControlBlock>>,
    /// 表示当前处理器上的 idle 控制流的任务上下文的地址。
    idle_task_cx: TaskContext,
}

impl Processor {
    pub fn new() -> Self {
        Self {
            current: None,
            idle_task_cx: TaskContext::zero_init(),
        }
    }
    fn get_idle_task_cx_ptr(&mut self) -> *mut TaskContext {
        &mut self.idle_task_cx as *mut _
    }
    //Processor::take_current 可以取出当前正在执行的任务。Option::take 意味着 current 字段也变为 None 。
    pub fn take_current(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.current.take()
    }
    //Processor::current返回当前执行的任务的一份拷贝。
    pub fn current(&self) -> Option<Arc<TaskControlBlock>> {
        self.current.as_ref().map(|task| Arc::clone(task))
    }
}

//在单核环境下，我们仅创建单个 Processor 的全局实例 PROCESSOR
lazy_static! {
    /// PROCESSOR instance through lazy_static!
    pub static ref PROCESSOR: UPSafeCell<Processor> = unsafe { UPSafeCell::new(Processor::new()) };
}

//每个 Processor 都有一个 idle 控制流，它们运行在每个核各自的启动栈上，
//功能是尝试从任务管理器中选出一个任务来在当前核上执行。
// 在内核初始化完毕之后，核通过调用 run_tasks 函数来进入 idle 控制流
///流程执行和调度的主要部分
//它循环调用 fetch_task 直到顺利从任务管理器中取出一个任务，然后获得 __switch 两个参数进行任务切换。
//注意在整个过程中要严格控制临界区。
pub fn run_tasks() {
    loop {
        let mut processor = PROCESSOR.exclusive_access();
        if let Some(task) = fetch_task() {
            let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
            // access coming task TCB exclusively
            let mut task_inner = task.inner_exclusive_access();
            let next_task_cx_ptr = &task_inner.task_cx as *const TaskContext;
            task_inner.task_status = TaskStatus::Running;
            drop(task_inner);
            // release coming task TCB manually
            processor.current = Some(task);
            // release processor manually
            drop(processor);
            unsafe {
                __switch(idle_task_cx_ptr, next_task_cx_ptr);
            }
        }
    }
}

/// Get current task through take, leaving a None in its place
pub fn take_current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().take_current()
}

/// Get a copy of the current task
pub fn current_task() -> Option<Arc<TaskControlBlock>> {
    PROCESSOR.exclusive_access().current()
}

/// Get token of the address space of current task
pub fn current_user_token() -> usize {
    let task = current_task().unwrap();
    let token = task.inner_exclusive_access().get_user_token();
    token
}

/// Get the mutable reference to trap context of current task
pub fn current_trap_cx() -> &'static mut TrapContext {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .get_trap_cx()
}

/// Return to idle control flow for new scheduling
//当一个应用交出 CPU 使用权时，进入内核后它会调用 schedule 函数来切换到 idle 控制流并开启新一轮的任务调度。
//切换回去之后，我们将跳转到 Processor::run 中 __switch 返回之后的位置，也即开启了下一轮循环。
pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let mut processor = PROCESSOR.exclusive_access();
    let idle_task_cx_ptr = processor.get_idle_task_cx_ptr();
    drop(processor);
    unsafe {
        __switch(switched_task_cx_ptr, idle_task_cx_ptr);
    }
}


//更新系统调用次数
pub fn update_syscall_times(id: usize) {
    let task = current_task().unwrap();
    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    inner.syscall_times[id] += 1;
}

//得到系统调用次数
pub fn get_syscall_times() -> [u32; config::MAX_SYSCALL_NUM] {
    current_task().unwrap().inner_exclusive_access().syscall_times
}

//得到进程运行时间
pub fn get_run_time() -> usize {
    let start_time = current_task().unwrap().inner_exclusive_access().start_time;
    timer::get_time_us() - start_time
}

//设置优先级
pub fn set_priority(_prio: isize) -> isize {
    if _prio < 2 {
        return -1;
    } else {
        current_task().unwrap().inner_exclusive_access().priority = _prio as u8;
        return _prio;
    }
}

//申请内存
pub fn mmap(_start: usize, _len: usize, _port: usize) -> isize {
    if (_start % config::PAGE_SIZE != 0) || (_port & !0x7 != 0) || (_port & 0x7 == 0) {
        return -1;
    }
    let start_address = mm::VirtAddr(_start);
    let end_address = mm::VirtAddr(_start + _len);

    let map_permission = mm::MapPermission::from_bits((_port as u8) << 1).unwrap() | mm::MapPermission::U;

    for vpn in mm::VPNRange::new(mm::VirtPageNum::from(start_address), end_address.ceil()) {
        if let Some(pte) = current_task()
            .unwrap()
            .inner_exclusive_access()
            .memory_set
            .translate(vpn) {
            if pte.is_valid() {
                return -1;
            }
        };
    }

    current_task()
        .unwrap()
        .inner_exclusive_access()
        .memory_set
        .insert_framed_area(start_address, end_address, map_permission);

    0
}

//释放内存
pub fn munmap(_start: usize, _len: usize) -> isize {
    if _start % config::PAGE_SIZE != 0 {
        return -1;
    }

    let start_address = mm::VirtAddr(_start);
    let end_address = mm::VirtAddr(_start + _len);

    for vpn in mm::VPNRange::new(mm::VirtPageNum::from(start_address), end_address.ceil()) {
        match current_task()
            .unwrap()
            .inner_exclusive_access()
            .memory_set
            .translate(vpn) {
            Some(pte) => {
                if pte.is_valid() == false {
                    return -1;
                }
            }
            None => {
                return -1;
            }
        }
    }

    for vpn in mm::VPNRange::new(mm::VirtPageNum::from(start_address), end_address.ceil()) {
        current_task().unwrap().inner_exclusive_access().memory_set.remove_area_with_start_vpn(vpn);
    }

    0
}