//[`TaskManager`]的实现
//它仅用于基于就绪队列管理流程和调度流程。
//其他CPU进程监控功能在处理器中。

use super::TaskControlBlock;
use crate::config;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;

//TaskManager 将所有的任务控制块用引用计数 Arc 智能指针包裹后放在一个双端队列 VecDeque 中。 
//使用智能指针的原因在于，任务控制块经常需要被放入/取出，如果直接移动任务控制块自身将会带来大量的数据拷贝开销， 
//而对于智能指针进行移动则没有多少开销。
//其次，允许任务控制块的共享引用在某些情况下能够让我们的实现更加方便。
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

// YOUR JOB: FIFO->Stride
/// A simple FIFO scheduler.
impl TaskManager {
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    ///将进程添加回就绪队列
    //TaskManager 提供 add/fetch 两个操作，前者表示将一个任务加入队尾，后者则表示从队头中取出一个任务来执行。 
    //从调度算法来看，这里用到的就是最简单的 RR 算法。
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    ///将进程从就绪队列中取出
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        let mut min_pass: usize = usize::MAX;
        let mut idx = 0;
        for i in 0..self.ready_queue.len() {
            let task = &self.ready_queue[i];
            let inner = task.inner_exclusive_access();
            if i == 0 {
                min_pass = inner.pass;
                idx = i;
            } else {
                if ((inner.pass - min_pass) as i8) < 0 {
                    min_pass = inner.pass;
                    idx = i;
                }
            }
            drop(inner);
            drop(task);
        }
        let task = &self.ready_queue[idx];
        let mut inner = task.inner_exclusive_access();
        let stride: u8 = (config::BIG_STRIDE as u8) / inner.priority;
        inner.pass += stride as usize;
        drop(inner);
        drop(task);
        self.ready_queue.remove(idx)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

//全局实例 TASK_MANAGER 提供给内核的其他子模块 add_task/fetch_task 两个函数。
pub fn add_task(task: Arc<TaskControlBlock>) {
    TASK_MANAGER.exclusive_access().add(task);
}

pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    TASK_MANAGER.exclusive_access().fetch()
}
