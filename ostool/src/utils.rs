//! Common utilities and helper functions.
//!
//! This module provides utility types and functions used throughout ostool,
//! including command execution helpers and string processing utilities.

use std::{
    ffi::OsStr,
    ops::{Deref, DerefMut},
    path::Path,
};

use anyhow::bail;
use colored::Colorize;

/// A command builder wrapper with variable substitution support.
///
/// `Command` wraps `std::process::Command` and adds support for automatic
/// variable replacement in arguments and environment values.
pub struct Command {
    inner: std::process::Command,
    value_replace: Box<dyn Fn(&OsStr) -> String>,
}

impl Deref for Command {
    type Target = std::process::Command;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Command {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Command {
    /// Creates a new command builder.
    ///
    /// # Arguments
    ///
    /// * `program` - The program to execute.
    /// * `workdir` - The working directory for the command.
    /// * `value_replace` - Function to perform variable substitution on arguments.
    pub fn new<S>(
        program: S,
        workdir: &Path,
        value_replace: impl Fn(&OsStr) -> String + 'static,
    ) -> Command
    where
        S: AsRef<OsStr>,
    {
        let mut cmd = std::process::Command::new(program);
        cmd.current_dir(workdir);
        cmd.env("WORKSPACE_FOLDER", workdir.display().to_string());

        Self {
            inner: cmd,
            value_replace: Box::new(value_replace),
        }
    }

    /// Prints the command to stdout with colored formatting.
    pub fn print_cmd(&self) {
        let mut cmd_str = self.get_program().to_string_lossy().to_string();

        for arg in self.get_args() {
            cmd_str += " ";
            cmd_str += arg.to_string_lossy().as_ref();
        }

        println!("{}", cmd_str.purple().bold());
    }

    /// Executes the command and waits for it to complete.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or exits with non-zero status.
    pub fn run(&mut self) -> anyhow::Result<()> {
        self.print_cmd();
        let status = self.status()?;
        if !status.success() {
            bail!("failed with status: {status}");
        }
        Ok(())
    }

    /// Adds an argument to the command with variable substitution.
    pub fn arg<S>(&mut self, arg: S) -> &mut Command
    where
        S: AsRef<OsStr>,
    {
        let value = (self.value_replace)(arg.as_ref());
        self.inner.arg(value);
        self
    }

    /// Adds multiple arguments to the command with variable substitution.
    pub fn args<I, S>(&mut self, args: I) -> &mut Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        for arg in args {
            self.arg(arg.as_ref());
        }
        self
    }

    /// Sets an environment variable for the command with variable substitution.
    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Command
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        let value = (self.value_replace)(val.as_ref());
        self.inner.env(key, value);
        self
    }
}

// pub async fn prepare_config<'de, C: JsonSchema + Deserialize<'de>>(
//     ctx: &mut AppContext,
//     config_path: Option<PathBuf>,
//     config_name: &str,
//     content: &'de mut String,
// ) -> anyhow::Result<C> {
//     // Implementation here
//     // Build logic will be implemented here
//     let config_path = match config_path {
//         Some(path) => path,
//         None => ctx.manifest_dir.join(config_name),
//     };

//     let schema_path = default_schema_by_init(&config_path);

//     let schema = schemars::schema_for!(C);
//     let schema_json = serde_json::to_value(&schema)?;
//     let schema_content = serde_json::to_string_pretty(&schema_json)?;
//     fs::write(&schema_path, schema_content).await?;

//     // 初始化AppData
//     // let app_data = AppData::new(Some(&config_path), Some(schema_path))?;

//     *content = fs::read_to_string(&config_path)
//         .await
//         .map_err(|_| anyhow!("can not open config file: {}", config_path.display()))?;

//     let config = match config_path
//         .extension()
//         .and_then(|s| s.to_str())
//         .unwrap_or("")
//     {
//         "json" => match serde_json::from_str::<C>(content){
//             Ok(v) => v,
//             Err(e) =>{
//                 warn!("Failed to parse JSON config: {}", e);
//             },
//         },
//         "toml" => toml::from_str::<C>(content)?,
//         _ => bail!(
//             "unsupported config file extension: {}",
//             config_path.display()
//         ),
//     };

//     Ok(config)
// }

/// Replaces environment variable placeholders in a string.
///
/// Placeholders use the format `${env:VAR_NAME}` where `VAR_NAME` is the
/// name of an environment variable. If the variable is not set, the
/// placeholder is replaced with an empty string.
///
/// # Example
///
/// ```rust
/// use ostool::utils::replace_env_placeholders;
///
/// unsafe { std::env::set_var("MY_VAR", "hello"); }
/// let result = replace_env_placeholders("Value: ${env:MY_VAR}").unwrap();
/// assert_eq!(result, "Value: hello");
/// ```
pub fn replace_env_placeholders(input: &str) -> anyhow::Result<String> {
    use std::env;

    // 使用正则表达式匹配 ${env:VAR_NAME} 格式
    // 由于我们要避免外部依赖，使用简单的字符串解析
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            // 开始可能的占位符
            chars.next(); // 消耗 '{'
            let mut placeholder = String::new();
            let mut brace_count = 1;
            let mut found_closing_brace = false;

            // 收集占位符内容
            for ch in chars.by_ref() {
                if ch == '{' {
                    brace_count += 1;
                    placeholder.push(ch);
                } else if ch == '}' {
                    brace_count -= 1;
                    if brace_count == 0 {
                        found_closing_brace = true;
                        break;
                    } else {
                        placeholder.push(ch);
                    }
                } else {
                    placeholder.push(ch);
                }
            }

            // 只有找到完整的占位符才进行处理
            if found_closing_brace && placeholder.starts_with("env:") {
                let env_var_name = &placeholder[4..]; // 跳过 "env:"

                // 获取环境变量值，如果不存在则替换为空字符串
                match env::var(env_var_name) {
                    Ok(value) => {
                        println!("Using {env_var_name}={value}");
                        result.push_str(&value)
                    }
                    Err(_) => {
                        // 环境变量不存在时替换为空字符串，不返回错误
                        result.push_str("");
                    }
                }
            } else {
                // 不是完整的占位符或不是环境变量占位符，保持原样
                result.push_str("${");
                result.push_str(&placeholder);
                if found_closing_brace {
                    result.push('}');
                }
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_replace_env_placeholders() {
        // 设置测试环境变量
        unsafe {
            env::set_var("TEST_HOME", "/home/test");
            env::set_var("TEST_PATH", "/usr/local/bin");
        }

        // 测试简单的环境变量替换
        assert_eq!(
            replace_env_placeholders("${env:TEST_HOME}").unwrap(),
            "/home/test"
        );

        // 测试多个环境变量
        assert_eq!(
            replace_env_placeholders("${env:TEST_HOME}:${env:TEST_PATH}").unwrap(),
            "/home/test:/usr/local/bin"
        );

        // 测试不存在的环境变量 - 应该返回空字符串而不是错误
        let result = replace_env_placeholders("${env:NON_EXISTENT}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");

        // 测试混合内容
        assert_eq!(
            replace_env_placeholders("Path: ${env:TEST_HOME}/bin").unwrap(),
            "Path: /home/test/bin"
        );

        // 测试非环境变量占位符
        assert_eq!(
            replace_env_placeholders("${not_env:placeholder}").unwrap(),
            "${not_env:placeholder}"
        );

        // 测试无占位符的字符串
        assert_eq!(
            replace_env_placeholders("Just a normal string").unwrap(),
            "Just a normal string"
        );

        // 测试空字符串
        assert_eq!(replace_env_placeholders("").unwrap(), "");
    }

    #[test]
    fn test_nested_braces() {
        unsafe {
            env::set_var("TEST_VAR", "value");
        }

        // 测试嵌套大括号的情况
        assert_eq!(
            replace_env_placeholders("${env:TEST_VAR} and ${other:placeholder}").unwrap(),
            "value and ${other:placeholder}"
        );
    }

    #[test]
    fn test_real_env_vars() {
        // 测试真实的环境变量（如果存在）
        if let Ok(home) = env::var("HOME") {
            assert_eq!(replace_env_placeholders("${env:HOME}").unwrap(), home);
        }
    }

    #[test]
    fn test_edge_cases() {
        // 测试不完整的占位符
        assert_eq!(replace_env_placeholders("${").unwrap(), "${");
        assert_eq!(replace_env_placeholders("${env").unwrap(), "${env");
        assert_eq!(replace_env_placeholders("${env:").unwrap(), "${env:");
        assert_eq!(replace_env_placeholders("${env:VAR").unwrap(), "${env:VAR");

        // 测试空的env变量名 - 应该返回空字符串而不是错误
        let result = replace_env_placeholders("${env:}");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "");

        // 测试只包含$的字符串
        assert_eq!(replace_env_placeholders("$").unwrap(), "$");
        assert_eq!(replace_env_placeholders("$$").unwrap(), "$$");

        // 测试包含特殊字符的环境变量名
        unsafe {
            env::set_var("TEST-VAR", "dash-value");
            env::set_var("TEST_VAR", "underscore-value");
        }
        assert_eq!(
            replace_env_placeholders("${env:TEST-VAR}").unwrap(),
            "dash-value"
        );
        assert_eq!(
            replace_env_placeholders("${env:TEST_VAR}").unwrap(),
            "underscore-value"
        );

        // 测试空的环境变量值
        unsafe {
            env::set_var("EMPTY_VAR", "");
        }
        assert_eq!(replace_env_placeholders("${env:EMPTY_VAR}").unwrap(), "");
    }

    #[test]
    fn test_malformed_placeholders() {
        // 测试格式错误的占位符
        assert_eq!(replace_env_placeholders("${env:VAR").unwrap(), "${env:VAR");
        assert_eq!(replace_env_placeholders("${env}").unwrap(), "${env}");
        assert_eq!(replace_env_placeholders("${:VAR}").unwrap(), "${:VAR}");

        // 设置测试环境变量
        unsafe {
            env::set_var("VAR", "value");
        }

        // 测试混合的大括号
        // 当遇到完整的占位符后停止，剩余字符由主循环继续处理
        assert_eq!(replace_env_placeholders("${env:VAR}}").unwrap(), "value}");

        // 测试其他格式错误的情况
        assert_eq!(replace_env_placeholders("{env:VAR}").unwrap(), "{env:VAR}");
        assert_eq!(replace_env_placeholders("$env:VAR}").unwrap(), "$env:VAR}");
    }
}
