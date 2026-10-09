//! A RISC-V hart as WCH's QingKe cores are: the V2A's RV32EC and the V4C's
//! RV32IMAC, each with Zicsr, machine mode only, and the two things that
//! are QingKe's own — a vector table of addresses rather than of jumps, and
//! a hardware stack that saves the caller-saved registers on the way into a
//! trap and puts them back on `mret`. Which core a hart is ([`Core`]) decides
//! how many registers it has and which extensions it decodes.
//!
//! **The hardware stack is the one that is not optional.** qingke-rt turns
//! it on (`csrw 0x804, 3`) and every handler it generates saves nothing but
//! `ra` before calling an ordinary Rust function, which is free to clobber
//! `t0`–`t2` and `a0`–`a5`. Without the hardware putting them back, the
//! first interrupt corrupts whatever the interrupted code was computing.
//! The silicon pushes them to memory; this keeps them beside the hart, which
//! is the same to every program that does not read its own stack below `sp`.
//!
//! **What a core does not have is refused, not provided.** The V2A has no M
//! extension, so `mul` is an illegal instruction there as it is on the chip
//! — a firmware built for the wrong target fails in the emulator the way it
//! would fail on the desk, rather than running in one and not the other.
//! Registers above `x15` are refused the same way: RV32E has sixteen. The
//! V4C decodes all thirty-two, the multiply and divide, and the atomics.

/// The width of one memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Size {
    Byte,
    Half,
    Word,
}

impl Size {
    pub fn bytes(self) -> u32 {
        match self {
            Size::Byte => 1,
            Size::Half => 2,
            Size::Word => 4,
        }
    }
}

/// Why the bus refused an access. The hart turns it into the matching
/// exception, with the address in `mtval`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fault;

/// Whatever is behind the hart's loads, stores and fetches.
pub trait Bus {
    /// A load: the value zero-extended to 32 bits.
    fn load(&mut self, addr: u32, size: Size) -> Result<u32, Fault>;
    /// A store of the low `size` bytes of `value`.
    fn store(&mut self, addr: u32, size: Size, value: u32) -> Result<(), Fault>;
    /// A fetch. Separate from `load` so reading code has no side effect a
    /// read of a peripheral register would have.
    fn fetch(&mut self, addr: u32) -> Result<u16, Fault>;
}

/// The exception codes this hart raises, as `mcause` carries them.
pub mod cause {
    pub const INSTRUCTION_MISALIGNED: u32 = 0;
    pub const INSTRUCTION_FAULT: u32 = 1;
    pub const ILLEGAL_INSTRUCTION: u32 = 2;
    pub const BREAKPOINT: u32 = 3;
    pub const LOAD_MISALIGNED: u32 = 4;
    pub const LOAD_FAULT: u32 = 5;
    pub const STORE_MISALIGNED: u32 = 6;
    pub const STORE_FAULT: u32 = 7;
    pub const MACHINE_ECALL: u32 = 11;

    /// What a code means, for the line that says a trap happened.
    pub fn name(code: u32) -> &'static str {
        match code {
            INSTRUCTION_MISALIGNED => "misaligned instruction",
            INSTRUCTION_FAULT => "instruction fetch from an unmapped address",
            ILLEGAL_INSTRUCTION => "illegal instruction",
            BREAKPOINT => "breakpoint",
            LOAD_MISALIGNED => "misaligned load",
            LOAD_FAULT => "load from an unmapped address",
            STORE_MISALIGNED => "misaligned store",
            STORE_FAULT => "store to an unmapped address",
            MACHINE_ECALL => "ecall",
            _ => "exception",
        }
    }
}

/// Which QingKe core a hart is: what it decodes, and what it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Core {
    /// Sixteen for RV32E, thirty-two for RV32I.
    pub registers: usize,
    /// The M extension: multiply and divide.
    pub m: bool,
    /// The A extension: load-reserved, store-conditional and the AMOs.
    pub a: bool,
    /// `GINTENR` (CSR 0x800), the window onto `mstatus`'s MIE and MPIE that
    /// qingke's critical sections use on every core but the V2.
    pub gintenr: bool,
    /// Where SDI print's two debug data registers are: the V2's sit at
    /// 0xE00000F4, everything later's at 0xE0000380.
    pub debug_data: u32,
}

impl Core {
    /// The CH32V003's: RV32EC.
    pub const V2A: Core = Core {
        registers: 16,
        m: false,
        a: false,
        gintenr: false,
        debug_data: 0xE000_00F4,
    };
    /// The CH32X035's: RV32IMAC.
    pub const V4C: Core = Core {
        registers: 32,
        m: true,
        a: true,
        gintenr: true,
        debug_data: 0xE000_0380,
    };

    /// The registers the hardware stack keeps across a trap: every
    /// caller-saved register the core has.
    fn stacked(self) -> &'static [usize] {
        if self.registers == 16 {
            &[1, 5, 6, 7, 10, 11, 12, 13, 14, 15]
        } else {
            &[1, 5, 6, 7, 10, 11, 12, 13, 14, 15, 16, 17, 28, 29, 30, 31]
        }
    }

    fn misa(self) -> u32 {
        let letter = |c: u8| 1u32 << (c - b'a');
        (1 << 30)
            | letter(b'c')
            | if self.registers == 16 {
                letter(b'e')
            } else {
                letter(b'i')
            }
            | if self.m { letter(b'm') } else { 0 }
            | if self.a { letter(b'a') } else { 0 }
    }
}

/// The CSRs a QingKe program touches, by number.
pub mod csr {
    pub const MSTATUS: u16 = 0x300;
    pub const MISA: u16 = 0x301;
    pub const MTVEC: u16 = 0x305;
    pub const MSCRATCH: u16 = 0x340;
    pub const MEPC: u16 = 0x341;
    pub const MCAUSE: u16 = 0x342;
    pub const MTVAL: u16 = 0x343;
    /// QingKe's global interrupt enable: MIE and MPIE, at their own bits.
    pub const GINTENR: u16 = 0x800;
    /// QingKe's interrupt system control: bit 0 the hardware stack, bit 1
    /// nesting.
    pub const INTSYSCR: u16 = 0x804;
    /// QingKe's core configuration.
    pub const CORECFGR: u16 = 0xBC0;
    pub const MVENDORID: u16 = 0xF11;
    pub const MARCHID: u16 = 0xF12;
    pub const MIMPID: u16 = 0xF13;
    pub const MHARTID: u16 = 0xF14;
}

const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
const MSTATUS_MPP: u32 = 0b11 << 11;

/// `mstatus`'s bits `GINTENR` reads and writes.
const GINTENR_BITS: u32 = MSTATUS_MIE | MSTATUS_MPIE;

/// What one step did, beyond moving on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// An ordinary instruction.
    Ran,
    /// `wfi`: the machine may let time pass until something is pending.
    Wait,
    /// An exception was taken; the hart is in its handler.
    Trapped { cause: u32, pc: u32, tval: u32 },
    /// A trap returned.
    Returned,
}

/// One hart's architectural state.
#[derive(Debug, Clone)]
pub struct Hart {
    pub core: Core,
    pub x: [u32; 32],
    pub pc: u32,
    pub mstatus: u32,
    pub mtvec: u32,
    pub mscratch: u32,
    pub mepc: u32,
    pub mcause: u32,
    pub mtval: u32,
    pub intsyscr: u32,
    pub corecfgr: u32,
    /// CSRs nobody here gives a meaning to: written and read back, and named
    /// once by the machine (`unknown_csr`).
    other: Vec<(u16, u32)>,
    /// What the hardware stack holds, innermost last.
    stacked: Vec<[u32; 16]>,
    /// The address an `lr.w` reserved, until an `sc.w` or a trap.
    reservation: Option<u32>,
    /// The first CSR a program touched that this hart does not model, for
    /// the machine to say once.
    pub unknown_csr: Option<u16>,
}

impl Default for Hart {
    fn default() -> Self {
        Self::new()
    }
}

impl Hart {
    /// A V2A out of reset.
    pub fn new() -> Self {
        Self::with(Core::V2A)
    }

    /// The state out of reset: execution at address zero, where the flash's
    /// boot alias puts the image's first word.
    pub fn with(core: Core) -> Self {
        Self {
            core,
            x: [0; 32],
            pc: 0,
            // Machine mode, interrupts off.
            mstatus: MSTATUS_MPP,
            mtvec: 0,
            mscratch: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            intsyscr: 0,
            corecfgr: 0,
            other: Vec::new(),
            stacked: Vec::new(),
            reservation: None,
            unknown_csr: None,
        }
    }

    pub fn interrupts_enabled(&self) -> bool {
        self.mstatus & MSTATUS_MIE != 0
    }

    /// Fetch, decode and run one instruction.
    pub fn step(&mut self, bus: &mut impl Bus) -> Step {
        let pc = self.pc;
        if pc & 1 != 0 {
            return self.exception(bus, cause::INSTRUCTION_MISALIGNED, pc);
        }
        let Ok(low) = bus.fetch(pc) else {
            return self.exception(bus, cause::INSTRUCTION_FAULT, pc);
        };
        let (op, len, raw) = if low & 0b11 == 0b11 {
            let Ok(high) = bus.fetch(pc.wrapping_add(2)) else {
                return self.exception(bus, cause::INSTRUCTION_FAULT, pc.wrapping_add(2));
            };
            let word = u32::from(low) | (u32::from(high) << 16);
            (decode(word, self.core), 4, word)
        } else {
            (decode_compressed(low, self.core), 2, u32::from(low))
        };
        self.execute(bus, op, len, raw)
    }

    fn reg(&self, r: u8) -> u32 {
        self.x[usize::from(r)]
    }

    fn set(&mut self, r: u8, value: u32) {
        if r != 0 {
            self.x[usize::from(r)] = value;
        }
    }

    fn execute(&mut self, bus: &mut impl Bus, op: Op, len: u32, raw: u32) -> Step {
        let pc = self.pc;
        let next = pc.wrapping_add(len);
        match op {
            Op::Lui { rd, imm } => self.set(rd, imm),
            Op::Auipc { rd, imm } => self.set(rd, pc.wrapping_add(imm)),
            Op::Jal { rd, imm } => {
                self.set(rd, next);
                self.pc = pc.wrapping_add(imm as u32);
                return Step::Ran;
            }
            Op::Jalr { rd, rs1, imm } => {
                let target = self.reg(rs1).wrapping_add(imm as u32) & !1;
                self.set(rd, next);
                self.pc = target;
                return Step::Ran;
            }
            Op::Branch { cmp, rs1, rs2, imm } => {
                let (a, b) = (self.reg(rs1), self.reg(rs2));
                let taken = match cmp {
                    Cmp::Eq => a == b,
                    Cmp::Ne => a != b,
                    Cmp::Lt => (a as i32) < (b as i32),
                    Cmp::Ge => (a as i32) >= (b as i32),
                    Cmp::Ltu => a < b,
                    Cmp::Geu => a >= b,
                };
                if taken {
                    self.pc = pc.wrapping_add(imm as u32);
                    return Step::Ran;
                }
            }
            Op::Load {
                size,
                signed,
                rd,
                rs1,
                imm,
            } => {
                let addr = self.reg(rs1).wrapping_add(imm as u32);
                if !addr.is_multiple_of(size.bytes()) {
                    return self.exception(bus, cause::LOAD_MISALIGNED, addr);
                }
                let Ok(value) = bus.load(addr, size) else {
                    return self.exception(bus, cause::LOAD_FAULT, addr);
                };
                let value = match (size, signed) {
                    (Size::Byte, true) => value as u8 as i8 as i32 as u32,
                    (Size::Half, true) => value as u16 as i16 as i32 as u32,
                    _ => value,
                };
                self.set(rd, value);
            }
            Op::Store {
                size,
                rs1,
                rs2,
                imm,
            } => {
                let addr = self.reg(rs1).wrapping_add(imm as u32);
                if !addr.is_multiple_of(size.bytes()) {
                    return self.exception(bus, cause::STORE_MISALIGNED, addr);
                }
                if bus.store(addr, size, self.reg(rs2)).is_err() {
                    return self.exception(bus, cause::STORE_FAULT, addr);
                }
            }
            Op::AluImm { alu, rd, rs1, imm } => {
                let value = alu.apply(self.reg(rs1), imm as u32);
                self.set(rd, value);
            }
            Op::Alu { alu, rd, rs1, rs2 } => {
                let value = alu.apply(self.reg(rs1), self.reg(rs2));
                self.set(rd, value);
            }
            Op::Amo { amo, rd, rs1, rs2 } => {
                let addr = self.reg(rs1);
                if !addr.is_multiple_of(4) {
                    let code = if amo == Amo::Lr {
                        cause::LOAD_MISALIGNED
                    } else {
                        cause::STORE_MISALIGNED
                    };
                    return self.exception(bus, code, addr);
                }
                match amo {
                    Amo::Lr => {
                        let Ok(value) = bus.load(addr, Size::Word) else {
                            return self.exception(bus, cause::LOAD_FAULT, addr);
                        };
                        self.reservation = Some(addr);
                        self.set(rd, value);
                    }
                    Amo::Sc => {
                        let held = self.reservation.take() == Some(addr);
                        if held && bus.store(addr, Size::Word, self.reg(rs2)).is_err() {
                            return self.exception(bus, cause::STORE_FAULT, addr);
                        }
                        self.set(rd, u32::from(!held));
                    }
                    _ => {
                        // A fault in either half is an AMO's store fault.
                        let Ok(old) = bus.load(addr, Size::Word) else {
                            return self.exception(bus, cause::STORE_FAULT, addr);
                        };
                        let new = amo.apply(old, self.reg(rs2));
                        if bus.store(addr, Size::Word, new).is_err() {
                            return self.exception(bus, cause::STORE_FAULT, addr);
                        }
                        self.set(rd, old);
                    }
                }
            }
            Op::Fence => {}
            Op::Ecall => return self.exception(bus, cause::MACHINE_ECALL, 0),
            Op::Ebreak => return self.exception(bus, cause::BREAKPOINT, pc),
            Op::Mret => {
                self.pc = self.mepc;
                let mpie = self.mstatus & MSTATUS_MPIE != 0;
                self.mstatus = (self.mstatus & !MSTATUS_MIE)
                    | if mpie { MSTATUS_MIE } else { 0 }
                    | MSTATUS_MPIE;
                if let Some(saved) = self.stacked.pop() {
                    for (slot, &reg) in self.core.stacked().iter().enumerate() {
                        self.x[reg] = saved[slot];
                    }
                }
                return Step::Returned;
            }
            Op::Wfi => {
                self.pc = next;
                return Step::Wait;
            }
            Op::Csr {
                kind,
                rd,
                source,
                immediate,
                csr,
            } => {
                let operand = if immediate {
                    u32::from(source)
                } else {
                    self.reg(source)
                };
                let old = self.read_csr(csr);
                let new = match kind {
                    CsrKind::Write => Some(operand),
                    // A set or clear with nothing to set or clear does not
                    // write at all — which matters for read-only CSRs.
                    CsrKind::Set => (source != 0).then_some(old | operand),
                    CsrKind::Clear => (source != 0).then_some(old & !operand),
                };
                if let Some(new) = new {
                    self.write_csr(csr, new);
                }
                self.set(rd, old);
            }
            Op::Illegal => return self.exception(bus, cause::ILLEGAL_INSTRUCTION, raw),
        }
        self.pc = next;
        Step::Ran
    }

    fn read_csr(&mut self, number: u16) -> u32 {
        match number {
            csr::MSTATUS => self.mstatus,
            csr::MISA => self.core.misa(),
            csr::GINTENR if self.core.gintenr => self.mstatus & GINTENR_BITS,
            csr::MTVEC => self.mtvec,
            csr::MSCRATCH => self.mscratch,
            csr::MEPC => self.mepc,
            csr::MCAUSE => self.mcause,
            csr::MTVAL => self.mtval,
            csr::INTSYSCR => self.intsyscr,
            csr::CORECFGR => self.corecfgr,
            // WCH's JEDEC bank and id, as the parts report them.
            csr::MVENDORID => 0x0000_0489,
            csr::MARCHID | csr::MIMPID | csr::MHARTID => 0,
            other => self.other_csr(other),
        }
    }

    fn write_csr(&mut self, number: u16, value: u32) {
        match number {
            csr::MSTATUS => self.mstatus = value,
            csr::GINTENR if self.core.gintenr => {
                self.mstatus = (self.mstatus & !GINTENR_BITS) | (value & GINTENR_BITS);
            }
            csr::MTVEC => self.mtvec = value,
            csr::MSCRATCH => self.mscratch = value,
            csr::MEPC => self.mepc = value & !1,
            csr::MCAUSE => self.mcause = value,
            csr::MTVAL => self.mtval = value,
            csr::INTSYSCR => self.intsyscr = value,
            csr::CORECFGR => self.corecfgr = value,
            csr::MISA | csr::MVENDORID | csr::MARCHID | csr::MIMPID | csr::MHARTID => {}
            other => {
                self.other_csr(other);
                if let Some(slot) = self.other.iter_mut().find(|(n, _)| *n == other) {
                    slot.1 = value;
                }
            }
        }
    }

    fn other_csr(&mut self, number: u16) -> u32 {
        if let Some((_, value)) = self.other.iter().find(|(n, _)| *n == number) {
            return *value;
        }
        if self.unknown_csr.is_none() {
            self.unknown_csr = Some(number);
        }
        self.other.push((number, 0));
        0
    }

    fn exception(&mut self, bus: &mut impl Bus, code: u32, tval: u32) -> Step {
        let pc = self.pc;
        self.trap(bus, code, tval, None);
        Step::Trapped {
            cause: code,
            pc,
            tval,
        }
    }

    /// Take an interrupt now. The caller has decided it is enabled and wins.
    pub fn interrupt(&mut self, bus: &mut impl Bus, irq: u32) {
        self.trap(bus, 0x8000_0000 | irq, 0, Some(irq));
    }

    /// Enter the trap handler for `mcause` = `code`, saving what the hardware
    /// saves. A vectored `mtvec` sends an exception to entry 3, QingKe's
    /// single entry for every synchronous fault, and an interrupt to its own
    /// number; with bit 1 set the entry holds the handler's address.
    fn trap(&mut self, bus: &mut impl Bus, code: u32, tval: u32, irq: Option<u32>) {
        self.mepc = self.pc;
        self.mcause = code;
        self.mtval = tval;
        self.reservation = None;
        let mie = self.mstatus & MSTATUS_MIE != 0;
        self.mstatus = (self.mstatus & !(MSTATUS_MIE | MSTATUS_MPIE))
            | if mie { MSTATUS_MPIE } else { 0 }
            | MSTATUS_MPP;
        if self.intsyscr & 1 != 0 {
            let mut saved = [0; 16];
            for (slot, &reg) in self.core.stacked().iter().enumerate() {
                saved[slot] = self.x[reg];
            }
            self.stacked.push(saved);
        }
        let base = self.mtvec & !0b11;
        self.pc = if self.mtvec & 1 == 0 {
            base
        } else {
            let entry = base.wrapping_add(4 * irq.unwrap_or(3));
            if self.mtvec & 0b10 != 0 {
                // A table of addresses. An entry nobody can read leaves the
                // hart at an address that will fault on fetch, which says
                // more than settling on some other handler.
                bus.load(entry, Size::Word).unwrap_or(0xFFFF_FFFE) & !1
            } else {
                entry
            }
        };
    }
}

/// One decoded instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Lui {
        rd: u8,
        imm: u32,
    },
    Auipc {
        rd: u8,
        imm: u32,
    },
    Jal {
        rd: u8,
        imm: i32,
    },
    Jalr {
        rd: u8,
        rs1: u8,
        imm: i32,
    },
    Branch {
        cmp: Cmp,
        rs1: u8,
        rs2: u8,
        imm: i32,
    },
    Load {
        size: Size,
        signed: bool,
        rd: u8,
        rs1: u8,
        imm: i32,
    },
    Store {
        size: Size,
        rs1: u8,
        rs2: u8,
        imm: i32,
    },
    AluImm {
        alu: Alu,
        rd: u8,
        rs1: u8,
        imm: i32,
    },
    Alu {
        alu: Alu,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    Amo {
        amo: Amo,
        rd: u8,
        rs1: u8,
        rs2: u8,
    },
    Fence,
    Ecall,
    Ebreak,
    Mret,
    Wfi,
    Csr {
        kind: CsrKind,
        rd: u8,
        /// A register, or for the immediate forms a five-bit value.
        source: u8,
        immediate: bool,
        csr: u16,
    },
    Illegal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Ge,
    Ltu,
    Geu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Alu {
    Add,
    Sub,
    Sll,
    Slt,
    Sltu,
    Xor,
    Srl,
    Sra,
    Or,
    And,
    Mul,
    Mulh,
    Mulhsu,
    Mulhu,
    Div,
    Divu,
    Rem,
    Remu,
}

impl Alu {
    fn apply(self, a: u32, b: u32) -> u32 {
        let (sa, sb) = (a as i32, b as i32);
        match self {
            Alu::Add => a.wrapping_add(b),
            Alu::Sub => a.wrapping_sub(b),
            Alu::Sll => a << (b & 31),
            Alu::Slt => u32::from((a as i32) < (b as i32)),
            Alu::Sltu => u32::from(a < b),
            Alu::Xor => a ^ b,
            Alu::Srl => a >> (b & 31),
            Alu::Sra => ((a as i32) >> (b & 31)) as u32,
            Alu::Or => a | b,
            Alu::And => a & b,
            Alu::Mul => a.wrapping_mul(b),
            Alu::Mulh => ((i64::from(sa) * i64::from(sb)) >> 32) as u32,
            Alu::Mulhsu => ((i64::from(sa) * i64::from(b)) >> 32) as u32,
            Alu::Mulhu => ((u64::from(a) * u64::from(b)) >> 32) as u32,
            // Division by zero and the one overflow answer as the
            // specification says, without a trap: all ones, the dividend,
            // and the most negative number over minus one is itself.
            Alu::Div if b == 0 => u32::MAX,
            Alu::Div => sa.wrapping_div(sb) as u32,
            Alu::Divu if b == 0 => u32::MAX,
            Alu::Divu => a / b,
            Alu::Rem if b == 0 => a,
            Alu::Rem => sa.wrapping_rem(sb) as u32,
            Alu::Remu if b == 0 => a,
            Alu::Remu => a % b,
        }
    }
}

/// What an A-extension instruction does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Amo {
    Lr,
    Sc,
    Swap,
    Add,
    Xor,
    And,
    Or,
    Min,
    Max,
    Minu,
    Maxu,
}

impl Amo {
    fn apply(self, old: u32, operand: u32) -> u32 {
        match self {
            Amo::Swap | Amo::Lr | Amo::Sc => operand,
            Amo::Add => old.wrapping_add(operand),
            Amo::Xor => old ^ operand,
            Amo::And => old & operand,
            Amo::Or => old | operand,
            Amo::Min => (old as i32).min(operand as i32) as u32,
            Amo::Max => (old as i32).max(operand as i32) as u32,
            Amo::Minu => old.min(operand),
            Amo::Maxu => old.max(operand),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CsrKind {
    Write,
    Set,
    Clear,
}

fn decode(inst: u32, core: Core) -> Op {
    // RV32E has sixteen registers; an instruction naming another is illegal.
    let regs_ok = |regs: &[u32]| regs.iter().all(|&r| (r as usize) < core.registers);
    let opcode = inst & 0x7f;
    let rd = (inst >> 7) & 0x1f;
    let funct3 = (inst >> 12) & 0x7;
    let rs1 = (inst >> 15) & 0x1f;
    let rs2 = (inst >> 20) & 0x1f;
    let funct7 = inst >> 25;
    let imm_i = (inst as i32) >> 20;
    let imm_s = (((inst as i32) >> 25) << 5) | ((inst >> 7) & 0x1f) as i32;
    let imm_b = (((inst as i32) >> 31) << 12)
        | (((inst >> 7) & 1) << 11) as i32
        | (((inst >> 25) & 0x3f) << 5) as i32
        | (((inst >> 8) & 0xf) << 1) as i32;
    let imm_u = inst & 0xFFFF_F000;
    let imm_j = (((inst as i32) >> 31) << 20)
        | (inst & 0x000F_F000) as i32
        | (((inst >> 20) & 1) << 11) as i32
        | (((inst >> 21) & 0x3ff) << 1) as i32;
    let (rd8, rs18, rs28) = (rd as u8, rs1 as u8, rs2 as u8);

    match opcode {
        0x37 if regs_ok(&[rd]) => Op::Lui {
            rd: rd8,
            imm: imm_u,
        },
        0x17 if regs_ok(&[rd]) => Op::Auipc {
            rd: rd8,
            imm: imm_u,
        },
        0x6f if regs_ok(&[rd]) => Op::Jal {
            rd: rd8,
            imm: imm_j,
        },
        0x67 if funct3 == 0 && regs_ok(&[rd, rs1]) => Op::Jalr {
            rd: rd8,
            rs1: rs18,
            imm: imm_i,
        },
        0x63 if regs_ok(&[rs1, rs2]) => {
            let cmp = match funct3 {
                0 => Cmp::Eq,
                1 => Cmp::Ne,
                4 => Cmp::Lt,
                5 => Cmp::Ge,
                6 => Cmp::Ltu,
                7 => Cmp::Geu,
                _ => return Op::Illegal,
            };
            Op::Branch {
                cmp,
                rs1: rs18,
                rs2: rs28,
                imm: imm_b,
            }
        }
        0x03 if regs_ok(&[rd, rs1]) => {
            let (size, signed) = match funct3 {
                0 => (Size::Byte, true),
                1 => (Size::Half, true),
                2 => (Size::Word, false),
                4 => (Size::Byte, false),
                5 => (Size::Half, false),
                _ => return Op::Illegal,
            };
            Op::Load {
                size,
                signed,
                rd: rd8,
                rs1: rs18,
                imm: imm_i,
            }
        }
        0x23 if regs_ok(&[rs1, rs2]) => {
            let size = match funct3 {
                0 => Size::Byte,
                1 => Size::Half,
                2 => Size::Word,
                _ => return Op::Illegal,
            };
            Op::Store {
                size,
                rs1: rs18,
                rs2: rs28,
                imm: imm_s,
            }
        }
        0x13 if regs_ok(&[rd, rs1]) => {
            let alu = match (funct3, funct7) {
                (0, _) => Alu::Add,
                (2, _) => Alu::Slt,
                (3, _) => Alu::Sltu,
                (4, _) => Alu::Xor,
                (6, _) => Alu::Or,
                (7, _) => Alu::And,
                (1, 0) => Alu::Sll,
                (5, 0) => Alu::Srl,
                (5, 0x20) => Alu::Sra,
                _ => return Op::Illegal,
            };
            let imm = if matches!(funct3, 1 | 5) {
                rs2 as i32
            } else {
                imm_i
            };
            Op::AluImm {
                alu,
                rd: rd8,
                rs1: rs18,
                imm,
            }
        }
        0x33 if regs_ok(&[rd, rs1, rs2]) => {
            let alu = match (funct3, funct7) {
                (0, 0) => Alu::Add,
                (0, 0x20) => Alu::Sub,
                (1, 0) => Alu::Sll,
                (2, 0) => Alu::Slt,
                (3, 0) => Alu::Sltu,
                (4, 0) => Alu::Xor,
                (5, 0) => Alu::Srl,
                (5, 0x20) => Alu::Sra,
                (6, 0) => Alu::Or,
                (7, 0) => Alu::And,
                // funct7 1 is the M extension, which the V2A does not have.
                (0, 1) if core.m => Alu::Mul,
                (1, 1) if core.m => Alu::Mulh,
                (2, 1) if core.m => Alu::Mulhsu,
                (3, 1) if core.m => Alu::Mulhu,
                (4, 1) if core.m => Alu::Div,
                (5, 1) if core.m => Alu::Divu,
                (6, 1) if core.m => Alu::Rem,
                (7, 1) if core.m => Alu::Remu,
                _ => return Op::Illegal,
            };
            Op::Alu {
                alu,
                rd: rd8,
                rs1: rs18,
                rs2: rs28,
            }
        }
        0x2f if core.a && funct3 == 2 && regs_ok(&[rd, rs1, rs2]) => {
            let amo = match funct7 >> 2 {
                0x02 if rs2 == 0 => Amo::Lr,
                0x03 => Amo::Sc,
                0x01 => Amo::Swap,
                0x00 => Amo::Add,
                0x04 => Amo::Xor,
                0x0C => Amo::And,
                0x08 => Amo::Or,
                0x10 => Amo::Min,
                0x14 => Amo::Max,
                0x18 => Amo::Minu,
                0x1C => Amo::Maxu,
                _ => return Op::Illegal,
            };
            Op::Amo {
                amo,
                rd: rd8,
                rs1: rs18,
                rs2: rs28,
            }
        }
        0x0f if matches!(funct3, 0 | 1) => Op::Fence,
        0x73 => match funct3 {
            0 => match inst {
                0x0000_0073 => Op::Ecall,
                0x0010_0073 => Op::Ebreak,
                0x3020_0073 => Op::Mret,
                0x1050_0073 => Op::Wfi,
                _ => Op::Illegal,
            },
            1..=3 | 5..=7 if regs_ok(&[rd]) && (funct3 >= 5 || regs_ok(&[rs1])) => Op::Csr {
                kind: match funct3 & 0b11 {
                    1 => CsrKind::Write,
                    2 => CsrKind::Set,
                    _ => CsrKind::Clear,
                },
                rd: rd8,
                source: rs18,
                immediate: funct3 >= 5,
                csr: (inst >> 20) as u16,
            },
            _ => Op::Illegal,
        },
        _ => Op::Illegal,
    }
}

/// The register a three-bit compressed field names: x8 to x15.
fn creg(field: u16) -> u8 {
    8 + (field & 0b111) as u8
}

fn bit(inst: u16, n: u32) -> u32 {
    u32::from((inst >> n) & 1)
}

fn bits(inst: u16, high: u32, low: u32) -> u32 {
    u32::from(inst >> low) & ((1 << (high - low + 1)) - 1)
}

/// Sign-extend the low `width` bits.
fn sext(value: u32, width: u32) -> i32 {
    let shift = 32 - width;
    ((value << shift) as i32) >> shift
}

fn decode_compressed(inst: u16, core: Core) -> Op {
    if inst == 0 {
        return Op::Illegal;
    }
    // The full five-bit register fields name x16 and up, which RV32E has
    // not got.
    let limit = core.registers as u32;
    let funct3 = inst >> 13;
    let rd = bits(inst, 11, 7);
    let rs2 = bits(inst, 6, 2);
    let imm6 = sext((bit(inst, 12) << 5) | bits(inst, 6, 2), 6);
    match (inst & 0b11, funct3) {
        // C.ADDI4SPN
        (0, 0b000) => {
            let imm = (bits(inst, 12, 11) << 4)
                | (bits(inst, 10, 7) << 6)
                | (bit(inst, 6) << 2)
                | (bit(inst, 5) << 3);
            if imm == 0 {
                return Op::Illegal;
            }
            Op::AluImm {
                alu: Alu::Add,
                rd: creg(inst >> 2),
                rs1: 2,
                imm: imm as i32,
            }
        }
        // C.LW
        (0, 0b010) => Op::Load {
            size: Size::Word,
            signed: false,
            rd: creg(inst >> 2),
            rs1: creg(inst >> 7),
            imm: ((bits(inst, 12, 10) << 3) | (bit(inst, 6) << 2) | (bit(inst, 5) << 6)) as i32,
        },
        // C.SW
        (0, 0b110) => Op::Store {
            size: Size::Word,
            rs1: creg(inst >> 7),
            rs2: creg(inst >> 2),
            imm: ((bits(inst, 12, 10) << 3) | (bit(inst, 6) << 2) | (bit(inst, 5) << 6)) as i32,
        },
        // C.NOP / C.ADDI
        (1, 0b000) if rd < limit => Op::AluImm {
            alu: Alu::Add,
            rd: rd as u8,
            rs1: rd as u8,
            imm: imm6,
        },
        // C.JAL and C.J
        (1, 0b001) | (1, 0b101) => {
            let imm = (bit(inst, 12) << 11)
                | (bit(inst, 11) << 4)
                | (bits(inst, 10, 9) << 8)
                | (bit(inst, 8) << 10)
                | (bit(inst, 7) << 6)
                | (bit(inst, 6) << 7)
                | (bits(inst, 5, 3) << 1)
                | (bit(inst, 2) << 5);
            Op::Jal {
                rd: if funct3 == 0b001 { 1 } else { 0 },
                imm: sext(imm, 12),
            }
        }
        // C.LI
        (1, 0b010) if rd < limit => Op::AluImm {
            alu: Alu::Add,
            rd: rd as u8,
            rs1: 0,
            imm: imm6,
        },
        // C.ADDI16SP
        (1, 0b011) if rd == 2 => {
            let imm = (bit(inst, 12) << 9)
                | (bit(inst, 6) << 4)
                | (bit(inst, 5) << 6)
                | (bits(inst, 4, 3) << 7)
                | (bit(inst, 2) << 5);
            if imm == 0 {
                return Op::Illegal;
            }
            Op::AluImm {
                alu: Alu::Add,
                rd: 2,
                rs1: 2,
                imm: sext(imm, 10),
            }
        }
        // C.LUI
        (1, 0b011) if rd < limit && rd != 0 => {
            if imm6 == 0 {
                return Op::Illegal;
            }
            Op::Lui {
                rd: rd as u8,
                imm: (imm6 << 12) as u32,
            }
        }
        (1, 0b100) => {
            let rd = creg(inst >> 7);
            match bits(inst, 11, 10) {
                // C.SRLI and C.SRAI; a shift of 32 or more is RV64's.
                0b00 | 0b01 if bit(inst, 12) == 0 => Op::AluImm {
                    alu: if bits(inst, 11, 10) == 0 {
                        Alu::Srl
                    } else {
                        Alu::Sra
                    },
                    rd,
                    rs1: rd,
                    imm: rs2 as i32,
                },
                // C.ANDI
                0b10 => Op::AluImm {
                    alu: Alu::And,
                    rd,
                    rs1: rd,
                    imm: imm6,
                },
                0b11 if bit(inst, 12) == 0 => Op::Alu {
                    alu: match bits(inst, 6, 5) {
                        0b00 => Alu::Sub,
                        0b01 => Alu::Xor,
                        0b10 => Alu::Or,
                        _ => Alu::And,
                    },
                    rd,
                    rs1: rd,
                    rs2: creg(inst >> 2),
                },
                _ => Op::Illegal,
            }
        }
        // C.BEQZ and C.BNEZ
        (1, 0b110) | (1, 0b111) => {
            let imm = (bit(inst, 12) << 8)
                | (bits(inst, 11, 10) << 3)
                | (bits(inst, 6, 5) << 6)
                | (bits(inst, 4, 3) << 1)
                | (bit(inst, 2) << 5);
            Op::Branch {
                cmp: if funct3 == 0b110 { Cmp::Eq } else { Cmp::Ne },
                rs1: creg(inst >> 7),
                rs2: 0,
                imm: sext(imm, 9),
            }
        }
        // C.SLLI
        (2, 0b000) if rd < limit && bit(inst, 12) == 0 => Op::AluImm {
            alu: Alu::Sll,
            rd: rd as u8,
            rs1: rd as u8,
            imm: rs2 as i32,
        },
        // C.LWSP
        (2, 0b010) if rd < limit && rd != 0 => Op::Load {
            size: Size::Word,
            signed: false,
            rd: rd as u8,
            rs1: 2,
            imm: ((bit(inst, 12) << 5) | (bits(inst, 6, 4) << 2) | (bits(inst, 3, 2) << 6)) as i32,
        },
        (2, 0b100) if rd < limit && rs2 < limit => match (bit(inst, 12), rd, rs2) {
            (0, 0, _) => Op::Illegal,
            // C.JR
            (0, rs1, 0) => Op::Jalr {
                rd: 0,
                rs1: rs1 as u8,
                imm: 0,
            },
            // C.MV
            (0, rd, rs2) => Op::Alu {
                alu: Alu::Add,
                rd: rd as u8,
                rs1: 0,
                rs2: rs2 as u8,
            },
            (1, 0, 0) => Op::Ebreak,
            // C.JALR
            (1, rs1, 0) => Op::Jalr {
                rd: 1,
                rs1: rs1 as u8,
                imm: 0,
            },
            // C.ADD
            (_, rd, rs2) => Op::Alu {
                alu: Alu::Add,
                rd: rd as u8,
                rs1: rd as u8,
                rs2: rs2 as u8,
            },
        },
        // C.SWSP
        (2, 0b110) if rs2 < limit => Op::Store {
            size: Size::Word,
            rs1: 2,
            rs2: rs2 as u8,
            imm: ((bits(inst, 12, 9) << 2) | (bits(inst, 8, 7) << 6)) as i32,
        },
        _ => Op::Illegal,
    }
}

#[cfg(test)]
mod tests;
