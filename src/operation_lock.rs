use fs2::FileExt;
use std::error::Error;
use std::fs::{File, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

#[derive(Debug)]
pub struct OperationLock {
    _file: File,
}

pub fn acquire(codex_home: &Path) -> Result<OperationLock, Box<dyn Error>> {
    std::fs::create_dir_all(codex_home).map_err(|error| -> Box<dyn Error> {
        format!("创建 Codex 目录失败 {}：{error}", codex_home.display()).into()
    })?;
    let path = codex_home.join("qpp-operation.lock");
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(&path).map_err(|error| -> Box<dyn Error> {
        format!("打开操作锁文件失败 {}：{error}", path.display()).into()
    })?;
    file.try_lock_exclusive()
        .map_err(|error| -> Box<dyn Error> {
            format!("另一个 QuotaPlusPlus 实例正在操作，请等待其完成后重试：{error}").into()
        })?;
    Ok(OperationLock { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_a_second_process_level_operation() {
        let directory = tempdir().expect("tempdir");
        let first = acquire(directory.path()).expect("first lock");
        let error = acquire(directory.path()).expect_err("second lock should fail");
        assert!(error.to_string().contains("另一个 QuotaPlusPlus"));
        drop(first);
        acquire(directory.path()).expect("lock after release");
    }
}
