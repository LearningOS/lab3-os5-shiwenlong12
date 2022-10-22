
//任务管理实施
//关于任务管理的所有内容，如启动和切换任务都在这里实现。
//名为“TASK_MANAGER`”的[`TaskManager`]的单个全局实例控制操作系统中的所有任务。
//看到[`__switch`]时要小心。围绕此函数的控制流可能不是您所期望的。


mod context;
mod manager;
mod pid;
mod processor;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::get_app_data_by_name;
use alloc::sync::Arc;
use lazy_static::*;
use manager::fetch_task;
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;
pub use manager::add_task;
pub use pid::{pid_alloc, KernelStack, PidHandle};
pub use processor::{
    current_task, current_trap_cx, current_user_token, run_tasks, schedule, take_current_task,

    set_priority, mmap, munmap, update_syscall_times, get_run_time, get_syscall_times
};

/// 暂停当前任务，并切换到下一个任务
//当仅有一个任务的时候， suspend_current_and_run_next 的效果是会继续执行这个任务。
pub fn suspend_current_and_run_next() {
    // There must be an application running.
    //取出当前正在执行的任务
    let task = take_current_task().unwrap();

    // ---- access current TCB exclusively
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    //修改其进程控制块内的状态
    task_inner.task_status = TaskStatus::Ready;
    drop(task_inner);
    // ---- release current PCB

    // push back to ready queue.
    //将这个任务放入任务管理器的队尾
    add_task(task);
    // jump to scheduling cycle
    //调用 schedule 函数来触发调度并切换任务。
    schedule(task_cx_ptr);
}

/// Exit current task, recycle process resources and switch to the next task
//退出当前任务，回收进程资源并切换到下一个任务
pub fn exit_current_and_run_next(exit_code: i32) {
    // take from Processor
    //调用 take_current_task 来将当前进程控制块从处理器监控 PROCESSOR 中取出，
    //而不只是得到一份拷贝，这是为了正确维护进程控制块的引用计数
    let task = take_current_task().unwrap();
    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // Change status to Zombie
    //将进程控制块中的状态修改为 TaskStatus::Zombie 即僵尸进程
    inner.task_status = TaskStatus::Zombie;
    // Record exit code
    //将传入的退出码 exit_code 写入进程控制块中，后续父进程在 waitpid 的时候可以收集
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    //将当前进程的所有子进程挂在初始进程 initproc 下面
    {
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    // ++++++ release parent PCB

    //将当前进程的孩子向量清空
    inner.children.clear();
    // deallocate user space
    //对于当前进程占用的资源进行早期回收
    //MemorySet::recycle_data_pages 只是将地址空间中的逻辑段列表 areas 清空，
    //这将导致应用地址空间的所有数据被存放在的物理页帧被回收，而用来存放页表的那些物理页帧此时则不会被回收。
    inner.memory_set.recycle_data_pages();
    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    //调用 schedule 触发调度及任务切换，我们再也不会回到该进程的执行过程，因此无需关心任务上下文的保存。
    schedule(&mut _unused as *mut _);
}

//内核初始化完毕之后，即会调用 task 子模块提供的 add_initproc 函数来将初始进程 initproc 加入任务管理器，
//但在这之前，我们需要初始进程的进程控制块 INITPROC ，这基于 lazy_static 在运行时完成。
lazy_static! {
    /// Creation of initial process
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    //功能：调用 TaskControlBlock::new 来创建一个进程控制块，
    //参数：它需要传入 ELF 可执行文件的数据切片作为参数， 
    //这可以通过加载器 loader 子模块提供的 get_app_data_by_name 接口查找 initproc 的 ELF 数据来获得。
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new(TaskControlBlock::new(
        get_app_data_by_name("ch5b_initproc").unwrap()
    ));
}

//在初始化 INITPROC 之后，
//调用 task 的任务管理器 manager 子模块提供的 add_task 接口将进程控制块加入到任务管理器。
pub fn add_initproc() {
    add_task(INITPROC.clone());
}
