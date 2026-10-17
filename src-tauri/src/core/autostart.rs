//! 开机启动：把应用注册成当前用户登录 Windows 时自动运行的程序。
//!
//! 只用 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`（当前用户），
//! 不写系统级启动项：安装包结束页默认会以管理员身份启动一次应用，
//! 若沿用“先 HKLM 再回落 HKCU”的动态模式，就会写成所有用户共享的启动项，
//! 之后普通权限下的“关闭开机启动”会因为删不掉 HKLM 而静默失败。

/// 开机启动时附加的命令行参数：带上它就保持窗口隐藏，直接常驻托盘。
pub(crate) const AUTOSTART_FLAG: &str = "--autostart";

/// 注册表值名，同时也是任务管理器“启动”页里的条目名。
const APP_NAME: &str = "workbuddy-assistant";

/// 本次进程是否由开机启动拉起。
pub(crate) fn launched_by_autostart() -> bool {
    std::env::args_os().any(|argument| argument.to_str() == Some(AUTOSTART_FLAG))
}

#[cfg(windows)]
mod platform {
    use std::io;

    use auto_launch::{AutoLaunch, WindowsEnableMode};

    use super::{APP_NAME, AUTOSTART_FLAG};

    /// 读写启动项失败时的统一文案，避免把注册表细节暴露到界面。
    const ACCESS_ERROR: &str = "无法读写系统启动项，请检查注册表权限后重试";

    fn launcher() -> io::Result<AutoLaunch> {
        let path = std::env::current_exe()?;
        // auto-launch 0.6.0 是把“路径 + 参数”直接拼起来的，不给路径加引号；
        // 路径里一旦有空格（例如 Windows 用户名带空格），启动项就会指向错误的程序，
        // 所以这里自己补上引号。
        let quoted_path = format!("\"{}\"", path.display());
        Ok(AutoLaunch::new(
            APP_NAME,
            &quoted_path,
            WindowsEnableMode::CurrentUser,
            &[AUTOSTART_FLAG],
        ))
    }

    /// 启动项是否处于生效状态：“任务管理器里禁用”同样算未生效。
    pub(crate) fn is_enabled() -> Result<bool, String> {
        let launcher = launcher().map_err(|_| ACCESS_ERROR.to_owned())?;
        launcher.is_enabled().map_err(|_| ACCESS_ERROR.to_owned())
    }

    /// 打开或关闭启动项；关闭时同时清掉注册的启动项与任务管理器禁用位。
    pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
        let launcher = launcher().map_err(|_| ACCESS_ERROR.to_owned())?;
        let result = if enabled {
            launcher.enable()
        } else {
            launcher.disable()
        };
        result.map_err(|_| ACCESS_ERROR.to_owned())
    }
}

#[cfg(not(windows))]
mod platform {
    /// 非 Windows 平台没有统一的开机启动方式，交给收尾的启动项设置即可。
    pub(crate) fn is_enabled() -> Result<bool, String> {
        Ok(false)
    }

    pub(crate) fn set_enabled(_enabled: bool) -> Result<(), String> {
        Err("开机启动目前仅在 Windows 上可用".to_owned())
    }
}

pub(crate) use platform::{is_enabled, set_enabled};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_flag_is_the_registered_command_line_argument() {
        assert_eq!(AUTOSTART_FLAG, "--autostart");
    }
}
