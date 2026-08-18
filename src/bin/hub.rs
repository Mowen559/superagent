//! 大脑 A (Supervisor): 长生守护者
//! 负责维持原来的所有终端命令（CLI）、网络代理以及任务的总派发

fn main() {
    // 复用原有的终端调度逻辑，作为大脑A的主控入口
    hub_rs::cli::dispatch();
}
