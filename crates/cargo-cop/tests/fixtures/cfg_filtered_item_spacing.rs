//! The `binn` executable: process entry point, global allocator and panic
//! handler. All behaviour lives in the library crates; this crate only wires
//! them together.

#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

#[cfg(not(test))]
mod app;

#[cfg(not(test))]
mod memory_helpers;

#[cfg(not(test))]
use app::App;
#[cfg(not(test))]
use binn_rt::{LinuxSyscalls, MmapAllocator, PanicReport, RawProcess};
#[cfg(not(test))]
use core::{
    arch::global_asm,
    ffi::{c_int, c_void},
    panic::PanicInfo,
};

#[cfg(all(not(test), target_arch = "aarch64"))]
use core::ptr;

#[cfg(not(test))]
const URC_FATAL_PHASE1_ERROR: c_int = 3;
#[cfg(all(not(test), target_arch = "aarch64"))]
const MAX_AUXV_ENTRIES: usize = 1 << 20;

#[cfg(all(not(test), target_arch = "aarch64"))]
static mut AUXV_START: *const usize = ptr::null();
#[cfg(not(test))]
#[global_allocator]
static ALLOCATOR: MmapAllocator = MmapAllocator::new();

#[cfg(all(not(test), target_arch = "x86_64"))]
global_asm!(
    ".section .text.binn_entry,\"ax\",@progbits",
    ".global _start",
    ".type _start,@function",
    "_start:",
    "xor %ebp, %ebp",
    "mov %rsp, %rdi",
    "and $-16, %rsp",
    "call binn_start",
    "ud2",
    ".size _start, .-_start",
    options(att_syntax),
);

#[cfg(all(not(test), target_arch = "aarch64"))]
global_asm!(
    ".section .text.binn_entry,\"ax\",%progbits",
    ".global _start",
    ".type _start,%function",
    "_start:",
    "mov x0, sp",
    "bl binn_start",
    "brk #0",
    ".size _start, .-_start",
);

#[cfg(not(test))]
#[panic_handler]
fn on_panic(info: &PanicInfo<'_>) -> ! {
    PanicReport::emit(info)
}

// The prebuilt `core`/`alloc` carry unwind tables that name this routine even
// though `panic = "abort"` means Rust code never unwinds. Without LTO that
// reference survives to link time (risk R1), so binn defines it. Reporting a
// fatal phase-1 error makes any foreign unwind that reaches Rust frames abort
// instead of running through code that was not built to be unwound.
#[cfg(not(test))]
#[unsafe(no_mangle)]
extern "C" fn rust_eh_personality(
    _version: c_int,
    _actions: c_int,
    _exception_class: u64,
    _exception: *mut c_void,
    _context: *mut c_void,
) -> c_int {
    URC_FATAL_PHASE1_ERROR
}

// `panic = "abort"` prevents Rust unwinding, but prebuilt `alloc` retains an
// unwind-resume reference. Treat any attempted unwind as a fatal runtime error.
#[cfg(not(test))]
#[unsafe(no_mangle)]
extern "C" fn _Unwind_Resume(_exception: *mut c_void) -> ! {
    RawProcess::exit_immediately(URC_FATAL_PHASE1_ERROR)
}

#[cfg(all(not(test), target_arch = "aarch64"))]
unsafe fn initial_auxv(stack: *const usize) -> *const usize {
    if stack.is_null() {
        return ptr::null();
    }
    // SAFETY: `_start` passes the readable Linux initial process stack.
    let argc = unsafe { stack.read() };
    if argc > MAX_AUXV_ENTRIES {
        return ptr::null();
    }
    // SAFETY: the initial stack stores argv immediately after argc.
    let argv = unsafe { stack.add(1) };
    // SAFETY: Linux places the argv terminator after its `argc` pointers.
    if unsafe { argv.add(argc).read() } != 0 {
        return ptr::null();
    }
    // SAFETY: envp follows argv's null terminator.
    let envp = unsafe { argv.add(argc + 1) };
    for index in 0..MAX_AUXV_ENTRIES {
        // SAFETY: the initial stack contains an envp terminator before auxv.
        if unsafe { envp.add(index).read() } == 0 {
            // SAFETY: the auxiliary vector follows the envp terminator.
            return unsafe { envp.add(index + 1) };
        }
    }
    ptr::null()
}

#[cfg(all(not(test), target_arch = "aarch64"))]
#[unsafe(no_mangle)]
extern "C" fn getauxval(kind: usize) -> usize {
    // The compiler runtime uses this for optional AArch64 instruction paths.
    // Returning zero before startup selects the safe baseline implementation.
    let auxv = unsafe { AUXV_START };
    if auxv.is_null() {
        return 0;
    }
    for index in 0..MAX_AUXV_ENTRIES {
        let kind_offset = index * 2;
        // SAFETY: `AUXV_START` points to the kernel's terminated auxiliary vector.
        let found_kind = unsafe { auxv.add(kind_offset).read() };
        if found_kind == 0 {
            return 0;
        }
        if found_kind == kind {
            // SAFETY: each auxiliary vector entry consists of a kind/value pair.
            return unsafe { auxv.add(kind_offset + 1).read() };
        }
    }
    0
}

#[cfg(not(test))]
#[unsafe(no_mangle)]
extern "C" fn binn_start(initial_stack: *const usize) -> ! {
    if !LinuxSyscalls::install_sigpipe_ignore() {
        RawProcess::exit_immediately(1);
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: the entry stub passes the process-lifetime initial stack.
    unsafe {
        AUXV_START = initial_auxv(initial_stack);
    }
    // SAFETY: `_start` passes the Linux initial process stack, which remains
    // mapped and contains process-lifetime argument and environment strings.
    let Some(process) = (unsafe { RawProcess::from_initial_stack(initial_stack) }) else {
        RawProcess::exit_immediately(1);
    };
    RawProcess::exit_immediately(App::run(&process));
}
