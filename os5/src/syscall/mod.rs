// 系统调用的实现
// 只要用户空间希望使用`ecall`指令执行系统调用，就会调用所有系统调用的单一入口点[`syscall（）`]。
// 在这种情况下，处理器会引发一个“来自U模式的环境调用”异常，
// 该异常作为[`crat:：trap:：trap-handler`]中的一种情况处理。
// 为了清楚起见，每个系统调用都是作为自己的函数实现的，名为`sys_`，然后是系统调用的名称。
// 您可以在子模块中找到类似的函数，您还应该以这种方式实现系统调用。

const SYSCALL_READ: usize = 63;
const SYSCALL_WRITE: usize = 64;
const SYSCALL_EXIT: usize = 93;
const SYSCALL_YIELD: usize = 124;
const SYSCALL_GET_TIME: usize = 169;
const SYSCALL_GETPID: usize = 172;
const SYSCALL_FORK: usize = 220;
const SYSCALL_EXEC: usize = 221;
const SYSCALL_WAITPID: usize = 260;
const SYSCALL_SPAWN: usize = 400;
const SYSCALL_MUNMAP: usize = 215;
const SYSCALL_MMAP: usize = 222;
const SYSCALL_SET_PRIORITY: usize = 140;
const SYSCALL_TASK_INFO: usize = 410;

mod fs;
mod process;

use fs::*;
use process::*;
use crate::task;

/// 使用`syscall_id`和其他参数处理syscall异常
pub fn syscall(syscall_id: usize, args: [usize; 3]) -> isize {
    task::update_syscall_times(syscall_id);

    match syscall_id {
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_YIELD => sys_yield(),
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_FORK => sys_fork(),
        SYSCALL_EXEC => sys_exec(args[0] as *const u8),
        SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32),
        SYSCALL_GET_TIME => sys_get_time(args[0] as *mut TimeVal, args[1]),
        SYSCALL_MMAP => sys_mmap(args[0], args[1], args[2]),
        SYSCALL_MUNMAP => sys_munmap(args[0], args[1]),
        SYSCALL_SET_PRIORITY => sys_set_priority(args[0] as isize),
        SYSCALL_TASK_INFO => sys_task_info(args[0] as *mut TaskInfo),
        SYSCALL_SPAWN => sys_spawn(args[0] as *const u8),
        _ => panic!("Unsupported syscall_id: {}", syscall_id),
    }
}
