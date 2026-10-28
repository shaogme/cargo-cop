#[cfg(test)]
extern crate std;

use crate::LinuxSyscalls;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char};

#[cfg(test)]
use std::{path::PathBuf, process::Command};

#[cfg(all(test, target_arch = "aarch64"))]
use std::env;

pub(crate) const MAX_PROCESS_POINTERS: usize = 1 << 20;

/// Command-line arguments and environment from the Linux initial process stack.
///
/// Both are kept as raw byte strings: Linux places no encoding requirement
/// on either, and callers must not assume UTF-8.
#[derive(Debug, Clone)]
pub struct RawProcess {
    args: Vec<&'static CStr>,
    env: Vec<&'static CStr>,
}

#[cfg(test)]
pub(crate) struct CrossTestRunner;

#[cfg(test)]
impl CrossTestRunner {
    pub(crate) fn child(executable: PathBuf) -> Command {
        #[cfg(target_arch = "aarch64")]
        {
            let runner_variable = if cfg!(target_env = "musl") {
                "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUNNER"
            } else {
                "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER"
            };
            if let Some(runner) = env::var_os(runner_variable) {
                let runner = runner.to_string_lossy().into_owned();
                let mut parts = runner.split_whitespace();
                if let Some(program) = parts.next() {
                    let mut command = Command::new(program);
                    command.args(parts).arg(executable);
                    return command;
                }
            }
        }
        Command::new(executable)
    }
}

impl RawProcess {
    /// Parses arguments and environment from the Linux initial process stack.
    ///
    /// The stack contains `argc`, `argc` argument pointers, a null pointer,
    /// environment pointers, and another null pointer. Environment scanning
    /// is bounded to protect the bootstrap from a missing terminator.
    ///
    /// # Safety
    ///
    /// `stack` must point to a readable Linux initial stack. Its argument and
    /// environment pointers must refer to NUL-terminated strings valid for
    /// the process lifetime, and its environment terminator must occur within
    /// `MAX_PROCESS_POINTERS` entries.
    pub unsafe fn from_initial_stack(stack: *const usize) -> Option<Self> {
        if stack.is_null() {
            return None;
        }
        // SAFETY: the caller guarantees the initial stack starts with argc.
        let argc = unsafe { stack.read() };
        if argc > MAX_PROCESS_POINTERS {
            return None;
        }
        // SAFETY: the initial stack stores the argv array after argc.
        let argv = unsafe { stack.add(1).cast::<*const c_char>() };
        // SAFETY: the Linux stack includes argv's terminating null slot.
        if !unsafe { argv.add(argc).read() }.is_null() {
            return None;
        }
        let env_offset = argc.checked_add(1)?;
        // SAFETY: envp follows argv's terminator on the initial stack.
        let envp = unsafe { argv.add(env_offset) };
        let mut envc = None;
        for index in 0..MAX_PROCESS_POINTERS {
            // SAFETY: the caller guarantees the environment terminator occurs
            // within the defensive bound and all preceding stack slots exist.
            if unsafe { envp.add(index).read() }.is_null() {
                envc = Some(index);
                break;
            }
        }
        let envc = envc?;
        // SAFETY: the stack arrays and strings satisfy this function's contract.
        let args = unsafe { Self::collect(argv, argc)? };
        // SAFETY: the environment length was checked against its null slot.
        let env = unsafe { Self::collect(envp, envc)? };
        Some(Self { args, env })
    }

    /// Builds a process description from already-validated strings.
    pub fn from_parts(args: Vec<&'static CStr>, env: Vec<&'static CStr>) -> Self {
        Self { args, env }
    }

    /// All arguments, including `argv[0]`.
    pub fn args(&self) -> &[&'static CStr] {
        &self.args
    }

    /// The arguments after `argv[0]`.
    pub fn operands(&self) -> &[&'static CStr] {
        self.args.split_first().map_or(&[], |(_, rest)| rest)
    }

    /// Environment entries in `NAME=value` form, in the original order.
    pub fn env(&self) -> &[&'static CStr] {
        &self.env
    }

    /// The raw value of environment variable `name`. As with `getenv`, the
    /// first `NAME=value` entry wins; entries without `=` never match.
    pub fn var(&self, name: &[u8]) -> Option<&'static [u8]> {
        self.env.iter().copied().find_map(|entry| {
            let (key, rest) = entry.to_bytes().split_at_checked(name.len())?;
            if key == name {
                rest.strip_prefix(b"=")
            } else {
                None
            }
        })
    }

    /// The final path component of `argv[0]`, used to select multicall
    /// behaviour (`trash-put`, `trash-list`, ...). Empty when `argv[0]` is
    /// missing or ends with a slash.
    pub fn invoked_name(&self) -> &[u8] {
        let Some(first) = self.args.first() else {
            return &[];
        };
        let bytes = first.to_bytes();
        match bytes.iter().rposition(|&byte| byte == b'/') {
            Some(slash) => bytes.get(slash + 1..).unwrap_or(&[]),
            None => bytes,
        }
    }

    /// Terminates the process without flushing standard streams.
    pub fn exit_immediately(status: i32) -> ! {
        LinuxSyscalls::exit_group(status)
    }

    /// # Safety
    ///
    /// `list` must point to `length` readable pointers to NUL-terminated
    /// strings that remain valid for the process lifetime.
    unsafe fn collect(list: *const *const c_char, length: usize) -> Option<Vec<&'static CStr>> {
        let mut out = Vec::with_capacity(length);
        for index in 0..length {
            // SAFETY: the caller guarantees all `length` array slots are readable.
            let entry = unsafe { list.add(index).read() };
            if entry.is_null() {
                return None;
            }
            // SAFETY: the caller guarantees each non-null pointer names a
            // NUL-terminated process-lifetime string.
            out.push(unsafe { CStr::from_ptr(entry) });
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::{CrossTestRunner, MAX_PROCESS_POINTERS, RawProcess};
    use alloc::{vec, vec::Vec};
    use core::ffi::c_char;
    use std::env;

    fn stack(args: &[*const c_char], env: &[*const c_char]) -> Vec<usize> {
        let mut words = Vec::with_capacity(args.len() + env.len() + 3);
        words.push(args.len());
        words.extend(args.iter().map(|argument| *argument as usize));
        words.push(0);
        words.extend(env.iter().map(|entry| *entry as usize));
        words.push(0);
        words
    }

    #[test]
    fn reads_arguments_and_environment_from_initial_stack() {
        let words = stack(
            &[c"/usr/bin/trash-put".as_ptr(), c"a b".as_ptr()],
            &[c"HOME=/home/a".as_ptr()],
        );
        // SAFETY: the synthetic stack contains valid strings and both null terminators.
        let process = unsafe { RawProcess::from_initial_stack(words.as_ptr()) }.unwrap();
        assert_eq!(process.args(), [c"/usr/bin/trash-put", c"a b"]);
        assert_eq!(process.operands(), [c"a b"]);
        assert_eq!(process.env(), [c"HOME=/home/a"]);
        assert_eq!(process.invoked_name(), b"trash-put");
    }

    #[test]
    fn accepts_empty_lists_and_non_utf8_bytes() {
        static ARGUMENT: &[u8] = b"tool\xff\0";
        static VARIABLE: &[u8] = b"NAME=\xfe\0";
        let empty = [0usize, 0, 0];
        // SAFETY: the synthetic initial stack contains empty terminated arrays.
        let process = unsafe { RawProcess::from_initial_stack(empty.as_ptr()) }.unwrap();
        assert!(process.args().is_empty());
        assert!(process.env().is_empty());

        let words = stack(
            &[ARGUMENT.as_ptr().cast::<c_char>()],
            &[VARIABLE.as_ptr().cast::<c_char>()],
        );
        // SAFETY: both byte strings are static and NUL-terminated.
        let process = unsafe { RawProcess::from_initial_stack(words.as_ptr()) }.unwrap();
        assert_eq!(process.args()[0].to_bytes(), b"tool\xff");
        assert_eq!(process.var(b"NAME"), Some(&b"\xfe"[..]));
    }

    #[test]
    fn rejects_missing_terminators_and_oversized_argument_counts() {
        let missing_argv_terminator = [0usize, 1, 0];
        // SAFETY: the malformed test stack remains readable for the checked slots.
        let parsed = unsafe { RawProcess::from_initial_stack(missing_argv_terminator.as_ptr()) };
        assert!(parsed.is_none());

        let oversized_argc = [MAX_PROCESS_POINTERS + 1];
        // SAFETY: the parser rejects argc before reading any following slot.
        assert!(unsafe { RawProcess::from_initial_stack(oversized_argc.as_ptr()) }.is_none());

        let mut missing_env_terminator = vec![1usize; MAX_PROCESS_POINTERS + 2];
        missing_env_terminator[0] = 0;
        missing_env_terminator[1] = 0;
        // SAFETY: the full defensive scan remains inside this allocated buffer.
        let parsed = unsafe { RawProcess::from_initial_stack(missing_env_terminator.as_ptr()) };
        assert!(parsed.is_none());
    }

    #[test]
    fn looks_up_variables_by_exact_name() {
        let env = vec![
            c"HOMEDIR=/x",
            c"HOME=/home/a",
            c"BROKEN",
            c"HOME=/second",
            c"EMPTY=",
        ];
        let process = RawProcess::from_parts(vec![], env);
        assert_eq!(process.var(b"HOME"), Some(&b"/home/a"[..]));
        assert_eq!(process.var(b"EMPTY"), Some(&b""[..]));
        assert_eq!(process.var(b"BROKEN"), None);
        assert_eq!(process.var(b"HOM"), None);
        assert_eq!(process.var(b"MISSING"), None);
    }

    #[test]
    fn invoked_name_edge_cases() {
        assert_eq!(RawProcess::from_parts(vec![], vec![]).invoked_name(), b"");
        assert_eq!(
            RawProcess::from_parts(vec![c"dir/"], vec![]).invoked_name(),
            b""
        );
        assert!(RawProcess::from_parts(vec![], vec![]).operands().is_empty());
    }

    #[test]
    fn immediate_exit_uses_the_requested_status() {
        const CHILD_FLAG: &str = "BINN_RT_EXIT_CHILD";
        if env::var_os(CHILD_FLAG).is_some() {
            RawProcess::exit_immediately(37);
        }

        let executable = env::current_exe().expect("locate test executable");
        let output = CrossTestRunner::child(executable)
            .args([
                "--exact",
                "process::tests::immediate_exit_uses_the_requested_status",
            ])
            .env(CHILD_FLAG, "1")
            .output()
            .expect("run immediate-exit test child");
        assert_eq!(output.status.code(), Some(37));
    }
}
