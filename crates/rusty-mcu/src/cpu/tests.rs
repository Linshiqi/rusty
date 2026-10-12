use super::*;

/// Memory from address zero, nothing else.
struct Flat(Vec<u8>);

impl Flat {
    fn with(words: &[u32]) -> Self {
        let mut bytes = vec![0; 4096];
        for (i, word) in words.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        Flat(bytes)
    }

    fn halves(halves: &[u16]) -> Self {
        let mut bytes = vec![0; 4096];
        for (i, half) in halves.iter().enumerate() {
            bytes[i * 2..i * 2 + 2].copy_from_slice(&half.to_le_bytes());
        }
        Flat(bytes)
    }

    fn word(&self, addr: usize) -> u32 {
        u32::from_le_bytes(self.0[addr..addr + 4].try_into().unwrap())
    }
}

impl Bus for Flat {
    fn load(&mut self, addr: u32, size: Size) -> Result<u32, Fault> {
        let a = addr as usize;
        if a + size.bytes() as usize > self.0.len() {
            return Err(Fault);
        }
        let mut value = 0;
        for i in 0..size.bytes() as usize {
            value |= u32::from(self.0[a + i]) << (8 * i);
        }
        Ok(value)
    }

    fn store(&mut self, addr: u32, size: Size, value: u32) -> Result<(), Fault> {
        let a = addr as usize;
        if a + size.bytes() as usize > self.0.len() {
            return Err(Fault);
        }
        for i in 0..size.bytes() as usize {
            self.0[a + i] = (value >> (8 * i)) as u8;
        }
        Ok(())
    }

    fn fetch(&mut self, addr: u32) -> Result<u16, Fault> {
        self.load(addr, Size::Half).map(|v| v as u16)
    }
}

// The base encodings, written from the ISA manual's tables, so a test reads
// as the program it runs.
fn i_type(opcode: u32, rd: u32, f3: u32, rs1: u32, imm: i32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | opcode
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(0x13, rd, 0, rs1, imm)
}
fn lw(rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(0x03, rd, 2, rs1, imm)
}
fn lb(rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(0x03, rd, 0, rs1, imm)
}
fn sw(rs2: u32, rs1: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 5 & 0x7f) << 25) | (rs2 << 20) | (rs1 << 15) | (2 << 12) | ((imm & 0x1f) << 7) | 0x23
}
fn r_type(f7: u32, rs2: u32, rs1: u32, f3: u32, rd: u32) -> u32 {
    (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | 0x33
}
fn branch(f3: u32, rs1: u32, rs2: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 12 & 1) << 31)
        | ((imm >> 5 & 0x3f) << 25)
        | (rs2 << 20)
        | (rs1 << 15)
        | (f3 << 12)
        | ((imm >> 1 & 0xf) << 8)
        | ((imm >> 11 & 1) << 7)
        | 0x63
}
fn jal(rd: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 20 & 1) << 31)
        | ((imm >> 1 & 0x3ff) << 21)
        | ((imm >> 11 & 1) << 20)
        | ((imm >> 12 & 0xff) << 12)
        | (rd << 7)
        | 0x6f
}
fn lui(rd: u32, imm: u32) -> u32 {
    (imm & 0xFFFF_F000) | (rd << 7) | 0x37
}
fn csrrw(rd: u32, csr: u32, rs1: u32) -> u32 {
    (csr << 20) | (rs1 << 15) | (1 << 12) | (rd << 7) | 0x73
}
fn csrrs(rd: u32, csr: u32, rs1: u32) -> u32 {
    (csr << 20) | (rs1 << 15) | (2 << 12) | (rd << 7) | 0x73
}
const MRET: u32 = 0x3020_0073;
const ECALL: u32 = 0x0000_0073;

fn run(hart: &mut Hart, bus: &mut Flat, steps: usize) {
    for _ in 0..steps {
        hart.step(bus);
    }
}

#[test]
fn arithmetic_loads_and_stores_do_what_the_manual_says() {
    let mut bus = Flat::with(&[
        addi(1, 0, 100),
        addi(2, 0, -3),
        r_type(0, 2, 1, 0, 3),    // add x3 = 97
        r_type(0x20, 2, 1, 0, 4), // sub x4 = 103
        r_type(0, 2, 1, 3, 5),    // sltu: 100 < 0xffff_fffd
        r_type(0, 2, 1, 2, 6),    // slt: 100 < -3 is false
        r_type(0x20, 1, 2, 5, 7), // sra -3 >> (100 & 31) = -1
        lui(8, 0x1234_5000),
        sw(1, 0, 0x200),
        lw(9, 0, 0x200),
        addi(10, 0, -1),
        sw(10, 0, 0x204),
        lb(11, 0, 0x204),
    ]);
    let mut hart = Hart::new();
    run(&mut hart, &mut bus, 13);
    assert_eq!(hart.x[3], 97);
    assert_eq!(hart.x[4], 103);
    assert_eq!(hart.x[5], 1);
    assert_eq!(hart.x[6], 0);
    assert_eq!(hart.x[7], u32::MAX);
    assert_eq!(hart.x[8], 0x1234_5000);
    assert_eq!(hart.x[9], 100);
    assert_eq!(hart.x[11], u32::MAX, "lb sign-extends");
    assert_eq!(bus.word(0x200), 100);
}

#[test]
fn a_loop_counts_down_by_its_branch() {
    // x1 = 5; loop: x2 += 1; x1 -= 1; bne x1, x0, loop
    let mut bus = Flat::with(&[
        addi(1, 0, 5),
        addi(2, 2, 1),
        addi(1, 1, -1),
        branch(1, 1, 0, -8),
        jal(0, 0),
    ]);
    let mut hart = Hart::new();
    run(&mut hart, &mut bus, 1 + 3 * 5 + 3);
    assert_eq!(hart.x[2], 5);
    assert_eq!(hart.pc, 16, "parked on the jump to itself");
}

#[test]
fn x0_stays_zero_and_registers_above_x15_are_illegal() {
    let mut bus = Flat::with(&[addi(0, 0, 7), addi(16, 0, 1)]);
    let mut hart = Hart::new();
    hart.step(&mut bus);
    assert_eq!(hart.x[0], 0);
    let step = hart.step(&mut bus);
    assert_eq!(
        step,
        Step::Trapped {
            cause: cause::ILLEGAL_INSTRUCTION,
            pc: 4,
            tval: addi(16, 0, 1),
        }
    );
}

#[test]
fn the_m_extension_is_refused_as_the_chip_refuses_it() {
    // mul x3, x1, x2
    let mut bus = Flat::with(&[r_type(1, 2, 1, 0, 3)]);
    let mut hart = Hart::new();
    assert!(matches!(
        hart.step(&mut bus),
        Step::Trapped {
            cause: cause::ILLEGAL_INSTRUCTION,
            ..
        }
    ));
}

#[test]
fn compressed_instructions_decode_to_their_expansions() {
    // c.li x8, 5        010 0 01000 00101 01  = 0x4415
    // c.addi x8, -1     000 1 01000 11111 01  = 0x147D
    // c.mv x9, x8       100 0 01001 01000 10  = 0x84A2
    // c.add x9, x8      100 1 01001 01000 10  = 0x94A2
    // c.slli x9, 2      000 0 01001 00010 10  = 0x048A
    // c.srli x9', 1     100 0 00 001 00001 01 = 0x8085
    // c.lui x10, 1      011 0 01010 00001 01  = 0x6505
    let mut bus = Flat::halves(&[0x4415, 0x147D, 0x84A2, 0x94A2, 0x048A, 0x8085, 0x6505]);
    let mut hart = Hart::new();
    run(&mut hart, &mut bus, 2);
    assert_eq!(hart.x[8], 4);
    run(&mut hart, &mut bus, 2);
    assert_eq!(hart.x[9], 8);
    run(&mut hart, &mut bus, 1);
    assert_eq!(hart.x[9], 32);
    run(&mut hart, &mut bus, 1);
    assert_eq!(hart.x[9], 16);
    run(&mut hart, &mut bus, 1);
    assert_eq!(hart.x[10], 0x1000);
    assert_eq!(hart.pc, 14, "every one two bytes long");
}

#[test]
fn compressed_stack_access_and_jumps_land_where_they_should() {
    // c.addi16sp sp, -16   011 1 00010 11110 01 ... nzimm=-16: bit9=1,bit4=1,
    //   bit6=1? -16 = 0b11_1111_0000 (10 bits): [9]=1 [8:7]=11 [6]=1 [5]=1 [4]=1
    //   bits: 12=[9]=1, 6=[4]=1, 5=[6]=1, 4:3=[8:7]=11, 2=[5]=1 → 0x717D
    // c.swsp x8, 4(sp)     110 0001 00 01000 10 → offset 4: [5:2]=0001 in 12:9
    //   = 110 0 001 0 0 01000 10 = 0xC222
    // c.lwsp x9, 4(sp)     010 0 00100 ... offset 4: [4:2]=001 in 6:4 →
    //   010 0 01001 001 00 10 = 0x4492
    // c.j +4               101 ... imm=4: bit 3 of offset in bits 5:3 → [3:1]=010
    //   = 101 0 0 00 0 0 0 010 0 01 = 0xA011
    let mut bus = Flat::halves(&[0x717D, 0xC222, 0x4492, 0xA011, 0x0001, 0x0001]);
    let mut hart = Hart::new();
    hart.x[2] = 0x800;
    hart.x[8] = 0xabcd;
    run(&mut hart, &mut bus, 3);
    assert_eq!(hart.x[2], 0x800 - 16);
    assert_eq!(bus.word(0x800 - 12), 0xabcd);
    assert_eq!(hart.x[9], 0xabcd);
    hart.step(&mut bus);
    assert_eq!(hart.pc, 6 + 4);
}

#[test]
fn a_csr_set_with_x0_reads_without_writing() {
    let mut bus = Flat::with(&[
        addi(1, 0, 0x88),
        csrrw(0, 0x300, 1),
        csrrs(2, 0x300, 0),
        csrrs(3, 0x301, 0),
    ]);
    let mut hart = Hart::new();
    run(&mut hart, &mut bus, 4);
    assert_eq!(hart.x[2], 0x88);
    assert_eq!(hart.x[3] & (1 << 4), 1 << 4, "misa says E");
    assert_eq!(hart.x[3] & (1 << 12), 0, "and not M");
}

#[test]
fn a_vectored_address_table_sends_an_exception_to_entry_three() {
    // mtvec = 0x400 | 0b11; the table's entry 3 says 0x100.
    let mut bus = Flat::with(&[addi(1, 0, 0x403), csrrw(0, 0x305, 1), ECALL]);
    bus.store(0x40C, Size::Word, 0x100).unwrap();
    let mut hart = Hart::new();
    run(&mut hart, &mut bus, 2);
    let step = hart.step(&mut bus);
    assert!(matches!(
        step,
        Step::Trapped {
            cause: 11,
            pc: 8,
            ..
        }
    ));
    assert_eq!(hart.pc, 0x100);
    assert_eq!(hart.mepc, 8);
    assert_eq!(hart.mcause, 11);
}

#[test]
fn the_hardware_stack_gives_a_handler_s_clobbers_back() {
    // The handler at 0x100 clobbers a0 and t0 and returns; with INTSYSCR
    // bit 0 set the interrupted code must see its own values again.
    let mut bus = Flat::with(&[]);
    bus.store(0x100, Size::Word, addi(10, 0, 99)).unwrap();
    bus.store(0x104, Size::Word, addi(5, 0, 98)).unwrap();
    bus.store(0x108, Size::Word, MRET).unwrap();
    bus.store(0x40 + 4 * 12, Size::Word, 0x100).unwrap();
    let mut hart = Hart::new();
    hart.mtvec = 0x40 | 0b11;
    hart.intsyscr = 1;
    hart.mstatus |= MSTATUS_MIE;
    hart.x[10] = 1;
    hart.x[5] = 2;
    hart.pc = 0x20;
    hart.interrupt(&mut bus, 12);
    assert_eq!(hart.pc, 0x100);
    assert_eq!(hart.mcause, 0x8000_000C);
    assert!(!hart.interrupts_enabled(), "masked inside the handler");
    run(&mut hart, &mut bus, 3);
    assert_eq!(hart.pc, 0x20);
    assert_eq!((hart.x[10], hart.x[5]), (1, 2));
    assert!(hart.interrupts_enabled(), "MPIE back into MIE");
}

#[test]
fn without_the_hardware_stack_a_handler_s_clobbers_stay() {
    let mut bus = Flat::with(&[]);
    bus.store(0x100, Size::Word, addi(10, 0, 99)).unwrap();
    bus.store(0x104, Size::Word, MRET).unwrap();
    let mut hart = Hart::new();
    hart.mtvec = 0x100;
    hart.x[10] = 1;
    hart.interrupt(&mut bus, 20);
    run(&mut hart, &mut bus, 2);
    assert_eq!(hart.x[10], 99);
}

#[test]
fn an_unmapped_fetch_faults_with_its_address() {
    let mut bus = Flat::with(&[jal(0, 0x2000)]);
    let mut hart = Hart::new();
    hart.mtvec = 0x10;
    hart.step(&mut bus);
    let step = hart.step(&mut bus);
    assert_eq!(
        step,
        Step::Trapped {
            cause: cause::INSTRUCTION_FAULT,
            pc: 0x2000,
            tval: 0x2000,
        }
    );
}

/// An A-extension word: funct5, aq/rl clear, rs2, rs1, funct3 2, rd.
fn amo(funct5: u32, rd: u32, rs1: u32, rs2: u32) -> u32 {
    (funct5 << 27) | (rs2 << 20) | (rs1 << 15) | (2 << 12) | (rd << 7) | 0x2F
}

#[test]
fn the_v4c_multiplies_and_divides_as_the_specification_says() {
    let mut bus = Flat::with(&[
        addi(1, 0, -7),
        addi(2, 0, 3),
        r_type(1, 2, 1, 0, 3), // mul   -21
        r_type(1, 2, 1, 4, 4), // div   -2, toward zero
        r_type(1, 2, 1, 6, 5), // rem   -1, the dividend's sign
        r_type(1, 2, 1, 3, 6), // mulhu (2^32 - 7) * 3 >> 32 = 2
        r_type(1, 0, 1, 4, 7), // div by zero: all ones
        r_type(1, 0, 1, 6, 8), // rem by zero: the dividend
        r_type(1, 0, 1, 5, 9), // divu by zero: all ones
        lui(10, 0x8000_0000),
        addi(11, 0, -1),
        r_type(1, 11, 10, 4, 12), // the one overflow: MIN / -1 is MIN
        r_type(1, 11, 10, 6, 13), // and its remainder zero
        r_type(1, 2, 1, 1, 16),   // mulh -7 * 3: the high half of -21
    ]);
    let mut hart = Hart::with(Core::V4C);
    run(&mut hart, &mut bus, 14);
    assert_eq!(hart.x[3] as i32, -21);
    assert_eq!(hart.x[4] as i32, -2);
    assert_eq!(hart.x[5] as i32, -1);
    assert_eq!(hart.x[6], 2);
    assert_eq!(hart.x[7], u32::MAX);
    assert_eq!(hart.x[8] as i32, -7);
    assert_eq!(hart.x[9], u32::MAX);
    assert_eq!(hart.x[12], 0x8000_0000);
    assert_eq!(hart.x[13], 0);
    assert_eq!(hart.x[16], u32::MAX, "x16 exists on an RV32I core");
}

#[test]
fn the_v4c_has_thirty_two_registers_and_misa_says_imac() {
    let mut bus = Flat::with(&[addi(31, 0, 5), csrrs(30, 0x301, 0)]);
    let mut hart = Hart::with(Core::V4C);
    run(&mut hart, &mut bus, 2);
    assert_eq!(hart.x[31], 5);
    let misa = hart.x[30];
    for letter in *b"imac" {
        assert_ne!(misa & (1 << (letter - b'a')), 0, "{}", letter as char);
    }
    assert_eq!(misa & (1 << 4), 0, "not E");
}

/// An AMO returns the old word and stores the new; `sc.w` stores only
/// under the reservation `lr.w` made, and says which with zero or one.
#[test]
fn the_v4c_atomics_swap_add_and_hold_a_reservation() {
    let mut bus = Flat::with(&[
        addi(1, 0, 0x200),
        addi(2, 0, 5),
        sw(2, 1, 0),
        addi(3, 0, 3),
        amo(0x00, 4, 1, 3),  // amoadd.w x4, x3, (x1): x4 = 5, word = 8
        amo(0x01, 5, 1, 2),  // amoswap.w x5, x2, (x1): x5 = 8, word = 5
        amo(0x02, 6, 1, 0),  // lr.w x6, (x1): 5, reserved
        amo(0x03, 7, 1, 3),  // sc.w x7, x3, (x1): stores 3, x7 = 0
        amo(0x03, 8, 1, 2),  // sc.w again: no reservation, x8 = 1
        amo(0x10, 9, 1, 11), // amomin.w with x11 = 0: word = 0
    ]);
    let mut hart = Hart::with(Core::V4C);
    run(&mut hart, &mut bus, 10);
    assert_eq!((hart.x[4], hart.x[5], hart.x[6]), (5, 8, 5));
    assert_eq!((hart.x[7], hart.x[8]), (0, 1));
    assert_eq!(hart.x[9], 3);
    assert_eq!(bus.word(0x200), 0);
}

#[test]
fn the_v2a_refuses_the_atomics_and_gintenr_is_not_there() {
    let mut bus = Flat::with(&[amo(0x00, 4, 1, 3)]);
    let mut hart = Hart::new();
    assert!(matches!(
        hart.step(&mut bus),
        Step::Trapped {
            cause: cause::ILLEGAL_INSTRUCTION,
            ..
        }
    ));
    let mut bus = Flat::with(&[csrrs(1, 0x800, 0)]);
    let mut hart = Hart::new();
    hart.step(&mut bus);
    assert_eq!(hart.unknown_csr, Some(0x800), "named, not modelled");
}

/// qingke's critical section on a V4: `csrrc 0x800, 0x88` takes MIE and
/// MPIE away and hands back what they were; `csrs 0x800, 0x88` restores.
#[test]
fn gintenr_is_mstatus_mie_and_mpie_on_the_v4c() {
    let mut bus = Flat::with(&[
        addi(1, 0, 0x88),
        csrrs(0, 0x800, 1),
        (0x800 << 20) | (1 << 15) | (3 << 12) | (2 << 7) | 0x73, // csrrc x2, 0x800, x1
        csrrs(3, 0x300, 0),
    ]);
    let mut hart = Hart::with(Core::V4C);
    run(&mut hart, &mut bus, 4);
    assert_eq!(hart.x[2], 0x88, "both were on");
    assert_eq!(hart.x[3] & 0x88, 0, "and are off in mstatus");
    assert_eq!(hart.unknown_csr, None);
}

/// The V4C's hardware stack keeps the RV32I caller-saved registers too:
/// a6, a7 and t3..t6, which the V2A has not got.
#[test]
fn the_v4c_hardware_stack_keeps_the_registers_rv32e_lacks() {
    let mut bus = Flat::with(&[]);
    bus.store(0x100, Size::Word, addi(17, 0, 99)).unwrap();
    bus.store(0x104, Size::Word, addi(31, 0, 98)).unwrap();
    bus.store(0x108, Size::Word, MRET).unwrap();
    bus.store(0x40 + 4 * 12, Size::Word, 0x100).unwrap();
    let mut hart = Hart::with(Core::V4C);
    hart.mtvec = 0x40 | 0b11;
    hart.intsyscr = 1;
    hart.mstatus |= MSTATUS_MIE;
    hart.x[17] = 1;
    hart.x[31] = 2;
    hart.pc = 0x20;
    hart.interrupt(&mut bus, 12);
    run(&mut hart, &mut bus, 3);
    assert_eq!((hart.x[17], hart.x[31]), (1, 2));
}
