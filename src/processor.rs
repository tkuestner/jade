use std::collections::HashSet;

use rand::random;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const NUM_VARIABLE_REGISTERS: u8 = 16;
pub const FLAG_REGISTER_INDEX: u8 = 0xF;
pub const ROM_START_ADDR: u16 = 0x200;
pub const FONT_START_ADDR: u16 = 0x50;
pub const DISPLAY_WIDTH: u8 = 64;
pub const DISPLAY_HEIGHT: u8 = 32;
pub const MEMORY_SIZE: usize = 4096;
pub const NUM_FONT_CHARS: u8 = 16;
pub const BYTES_PER_CHAR: u8 = 5;

/// The core of the CHIP-8 emulator. Contains memory, stack, register and instructions execution.
#[derive(Default)]
pub struct Processor {
    program_data: Vec<u8>,
    settings: InstructionSettings,
    memory: Vec<u8>,
    stack: Vec<u16>,
    program_counter: u16,
    display: Vec<bool>,
    index_register: u16,
    variable_registers: [u8; NUM_VARIABLE_REGISTERS as usize],
    delay_timer: u8,
    sound_timer: u8,
    blocking: Option<BlockingState>,
    keys: HashSet<Key>,
}

impl Processor {
    const FONT: [u8; 80] = [
        0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
        0x20, 0x60, 0x20, 0x20, 0x70, // 1
        0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
        0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
        0x90, 0x90, 0xF0, 0x10, 0x10, // 4
        0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
        0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
        0xF0, 0x10, 0x20, 0x40, 0x40, // 7
        0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
        0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
        0xF0, 0x90, 0xF0, 0x90, 0x90, // A
        0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
        0xF0, 0x80, 0x80, 0x80, 0xF0, // C
        0xE0, 0x90, 0x90, 0x90, 0xE0, // D
        0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
        0xF0, 0x80, 0xF0, 0x80, 0x80, // F
    ];

    /// Create a new processor with default settings.
    pub fn new() -> Self {
        let mut memory = vec![0; MEMORY_SIZE];

        // Copy font to memory, from 0x50 to 0x9F (incl.) resp. A0 (excl.)
        let start = ROM_START_ADDR as usize;
        let end = start + BYTES_PER_CHAR as usize * NUM_FONT_CHARS as usize;
        memory[start..end].copy_from_slice(&Self::FONT);

        Processor {
            program_data: vec![],
            settings: InstructionSettings::default(),
            memory,
            stack: vec![],
            program_counter: ROM_START_ADDR,
            display: vec![false; DISPLAY_WIDTH as usize * DISPLAY_HEIGHT as usize],
            index_register: 0,
            variable_registers: [0; NUM_VARIABLE_REGISTERS as usize],
            delay_timer: 0,
            sound_timer: 0,
            blocking: None,
            keys: HashSet::new(),
        }
    }

    /// Configure how certain instructions are interpreted.
    pub fn load_settings(&mut self, settings: InstructionSettings) {
        self.settings = settings;
    }

    /// Reset the emulator (memory, registers, etc.) and load a ROM.
    pub fn load_program(&mut self, program_data: Vec<u8>) -> Result<(), EmulatorError> {
        let mut memory = vec![0; MEMORY_SIZE];

        // Copy font to memory, from 0x50 to 0x9F (incl.) resp. A0 (excl.)
        let start = ROM_START_ADDR as usize;
        let end = start + BYTES_PER_CHAR as usize * NUM_FONT_CHARS as usize;
        memory[start..end].copy_from_slice(&Self::FONT);

        // Copy rom to memory, starting at 0x0200
        let rom_size = program_data.len();
        if rom_size > memory.len() - ROM_START_ADDR as usize {
            return Err(EmulatorError::Loading(LoadingError::RomTooLarge));
        }
        let start = ROM_START_ADDR as usize;
        let end = start + rom_size;
        memory[start..end].copy_from_slice(&program_data);

        self.program_data = program_data;
        self.memory = memory;
        self.stack = vec![];
        self.program_counter = ROM_START_ADDR;
        self.display = vec![false; DISPLAY_WIDTH as usize * DISPLAY_HEIGHT as usize];
        self.index_register = 0;
        self.variable_registers = [0; NUM_VARIABLE_REGISTERS as usize];
        self.delay_timer = 0;
        self.sound_timer = 0;
        // settings stay unchanged

        Ok(())
    }

    /// Perform a single step, i.e. load the instruction at the program counter (PC) and
    /// execute it.
    pub fn step(&mut self) -> Result<(), EmulatorError> {
        // Fetch
        let address = self.program_counter;
        let pc = self.program_counter as usize;
        let instruction = u16::from_be_bytes([self.memory[pc], self.memory[pc + 1]]);
        self.program_counter += 2;

        self.execute(instruction)
            .map_err(|source| EmulatorError::Execution {
                address,
                instruction,
                source,
            })
    }

    /// Execute a single instruction, i.e. decode the opcode and act on it.
    fn execute(&mut self, instruction: u16) -> Result<(), ExecutionError> {
        // Decode
        let nibbles = [
            nibble(instruction, 0),
            nibble(instruction, 1),
            nibble(instruction, 2),
            nibble(instruction, 3),
        ];

        // Execute
        if instruction == 0x00e0 {
            // 00E0 - clear screen
            self.display = vec![false; DISPLAY_WIDTH as usize * DISPLAY_HEIGHT as usize];
        } else if instruction == 0x00ee {
            // 00EE - return from subroutine by popping the last address from the stack
            if let Some(address) = self.stack.pop() {
                self.program_counter = address;
            } else {
                return Err(ExecutionError::StackUnderflow);
            }
        } else if nibbles[0] == 1 {
            // 1NNN - jump NNN
            self.program_counter = instruction & 0x0fff;
        } else if nibbles[0] == 2 {
            // 2NNN - Call subroutine at NNN
            self.stack.push(self.program_counter);
            self.program_counter = instruction & 0x0fff;
        } else if nibbles[0] == 3 {
            // 3XNN - SKip one instruction if VX is equal to NN
            let register_index = nibbles[1] as usize;
            let register_value = self.variable_registers[register_index];
            let immediate_value = (instruction & 0x00ff) as u8;
            if register_value == immediate_value {
                self.program_counter += 2;
            }
        } else if nibbles[0] == 4 {
            // 4XNN - SKip one instruction if VX is not equal to NN
            let register_index = nibbles[1] as usize;
            let register_value = self.variable_registers[register_index];
            let immediate_value = (instruction & 0x00ff) as u8;
            if register_value != immediate_value {
                self.program_counter += 2;
            }
        } else if nibbles[0] == 5 {
            // 5Xy0 - SKip one instruction if VX is equal to VY
            let register_index_x = nibbles[1] as usize;
            let register_index_y = nibbles[2] as usize;
            let register_value_x = self.variable_registers[register_index_x];
            let register_value_y = self.variable_registers[register_index_y];
            if register_value_x == register_value_y {
                self.program_counter += 2;
            }
        } else if nibbles[0] == 6 {
            // 6XNN - set register VX to NN
            let register_index = nibbles[1] as usize;
            let value = (instruction & 0x00ff) as u8;
            self.variable_registers[register_index] = value;
        } else if nibbles[0] == 7 {
            // 7XNN - add value NN to register VX
            let register_index = nibbles[1] as usize;
            let value = (instruction & 0x00ff) as u8;
            self.variable_registers[register_index] = self.variable_registers[register_index]
                .overflowing_add(value)
                .0;
            // Ignore overflow indicator
        } else if nibbles[0] == 8 {
            // Arithmetic and logic instructions
            let register_index_x = nibbles[1];
            // let register_index_y = nibbles[2];
            let register_value_x = self.register(nibbles[1])?;
            let register_value_y = self.register(nibbles[2])?;

            if nibbles[3] == 0 {
                // 8XY0 - set VX to the value of VY
                self.set_register(register_index_x, register_value_y)?;
            } else if nibbles[3] == 1 {
                // 8XY1 - set VX to the bitwise OR of VX and VY
                self.set_register(register_index_x, register_value_x | register_value_y)?;
            } else if nibbles[3] == 2 {
                // 8XY2 - set VX to the bitwise AND of VX and VY
                self.set_register(register_index_x, register_value_x & register_value_y)?;
            } else if nibbles[3] == 3 {
                // 8XY3 - set VX to the bitwise XOR of VX and VY
                self.set_register(register_index_x, register_value_x ^ register_value_y)?;
            } else if nibbles[3] == 4 {
                // 8XY4 - set VX to the sum of VX and VY
                let (result, overflow) = register_value_x.overflowing_add(register_value_y);
                self.set_register(register_index_x, result)?;
                self.set_flag_register(overflow as u8);
            } else if nibbles[3] == 5 {
                // 8XY5 - set VX to the result of VX - VY
                let (result, overflow) = register_value_x.overflowing_sub(register_value_y);
                self.set_register(register_index_x, result)?;
                self.set_flag_register((!overflow) as u8);
            } else if nibbles[3] == 6 {
                // 8XY6 Shift the value of VX one bit to the right
                if self.settings.use_vy_in_8xy6 {
                    self.set_register(register_index_x, register_value_y)?;
                }
                let value = self.register(register_index_x)?;
                let lowest_bit = value & 0b1;
                let result = value >> 1;
                self.set_flag_register(lowest_bit);
                self.set_register(register_index_x, result)?;
            } else if nibbles[3] == 7 {
                // 8XY7 - set VX to the result of VY - VX
                let (result, overflow) = register_value_y.overflowing_sub(register_value_x);
                self.set_register(register_index_x, result)?;
                self.set_flag_register((!overflow) as u8);
            }
            // Note instructions ending with 8 to D are not defined in the instruction set.
            else if nibbles[3] == 0xE {
                // 8XY6 Shift the value of VX one bit to the left
                if self.settings.use_vy_in_8xye {
                    self.set_register(register_index_x, register_value_y)?;
                }
                let value = self.register(register_index_x)?;
                let highest_bit = value & 0b10000000; // == 0x80
                let flag = highest_bit >> 7;
                let result = value << 1;
                self.set_flag_register(flag);
                self.set_register(register_index_x, result)?;
            }
        } else if nibbles[0] == 9 {
            // 9Xy0 - SKip one instruction if VX is not equal to VY
            let register_index_x = nibbles[1] as usize;
            let register_index_y = nibbles[2] as usize;
            let register_value_x = self.variable_registers[register_index_x];
            let register_value_y = self.variable_registers[register_index_y];
            if register_value_x != register_value_y {
                self.program_counter += 2;
            }
        } else if nibbles[0] == 0xA {
            // ANNN - set index register to value NNN
            let value = instruction & 0x0fff;
            self.index_register = value;
        } else if nibbles[0] == 0xB {
            // BNNN - jump address NNN plus the value in V0
            let value = instruction & 0x0fff;
            if self.settings.use_bxnn_instead_bnnn {
                // BXNN
                let register_index_x = nibbles[1];
                let register_value_x = self.register(register_index_x)?;
                self.program_counter = value + register_value_x as u16;
            } else {
                self.program_counter = value + self.variable_registers[0] as u16;
            }
        } else if nibbles[0] == 0xC {
            // CXNN - generate random number, AND it with NN, store in VX
            let random_number: u8 = random();
            let value = (instruction & 0x00FF) as u8;
            //*self.register_mut(nibbles[1])? = random_number & value;
            self.set_register(nibbles[1], random_number & value)?;
        } else if nibbles[0] == 0xD {
            // DXYN - Draw an N pixels tall sprite from the memory location that the index register
            // is holding to the screen at the x coordinate in VX and y coordinate in VY.
            let register_index = nibbles[1] as usize;
            let dx = self.variable_registers[register_index];
            let register_index = nibbles[2] as usize;
            let dy = self.variable_registers[register_index];

            let rows = nibbles[3];
            // Take modulo operation on the x and y coordinates
            let mut dx = dx % DISPLAY_WIDTH;
            let dx_orig = dx;
            let mut dy = dy % DISPLAY_HEIGHT;

            // Clear VF
            self.variable_registers[0xf] = 0;

            // Loop over sprite rows, 1 sprite row = 1 byte = 8 pixels
            for n in 0..rows {
                let sprite_row = self.memory[self.index_register as usize + n as usize];
                for i in 0..8 {
                    let bit = sprite_row >> (7 - i) & 1;
                    let pixel =
                        &mut self.display[dy as usize * DISPLAY_WIDTH as usize + dx as usize];
                    if bit == 1 && *pixel {
                        *pixel = false;
                        self.variable_registers[0xf] = 1;
                    } else if bit == 1 && !(*pixel) {
                        *pixel = true;
                    }
                    dx += 1;
                    if dx >= DISPLAY_WIDTH {
                        break;
                    }
                }
                dy += 1;
                if dy >= DISPLAY_HEIGHT {
                    break;
                }
                dx = dx_orig;
            }
        } else if nibbles[0] == 0xE {
            if nibbles[2] == 0x9 && nibbles[3] == 0xE {
                // EX9E - Skip the next instruction if the key corresponding to the value in VX is currently pressed
                let value = self.register(nibbles[1])?;
                if let Ok(key) = Key::try_from(value) {
                    if self.keys.contains(&key) {
                        self.program_counter += 2;
                    }
                }
                // Maybe emit a warning if the value in register VX is > 16 and hence cannot be represented as a key
            } else if nibbles[2] == 0xA && nibbles[3] == 0x1 {
                // EXA1 - Skip the next instruction if the key corresponding to the value in VX is currently not pressed
                let value = self.register(nibbles[1])?;
                if let Ok(key) = Key::try_from(value) {
                    if !self.keys.contains(&key) {
                        self.program_counter += 2;
                    }
                }
                // Maybe emit a warning if the value in register VX is > 16 and hence cannot be represented as a key
            }
        } else if nibbles[0] == 0xF {
            if nibbles[2] == 0x0 && nibbles[3] == 0x7 {
                // FX07 - set VX to value of the delay timer
                self.set_register(nibbles[1], self.delay_timer)?;
            } else if nibbles[2] == 0x1 && nibbles[3] == 0x5 {
                // FX15 - set the delay timer to the value in VX
                self.delay_timer = self.register(nibbles[1])?;
            } else if nibbles[2] == 0x1 && nibbles[3] == 0x8 {
                // FX18 - set the sound timer to the value in VX
                self.sound_timer = self.register(nibbles[1])?;
            }
            if nibbles[2] == 0x1 && nibbles[3] == 0xE {
                // FX1E - add the value of VX to the index register
                let value = self.register(nibbles[1])?;
                self.index_register += value as u16;
                if self.settings.set_vf_on_overflow_in_fx1e {
                    // Note: not the overflow of u16, but addressing memory outside the common range,
                    // i.e. addresses above 0x0FFF.
                    if self.index_register >= MEMORY_SIZE as u16 {
                        self.set_register(0xF, 1)?;
                    }
                }
            }
            if nibbles[2] == 0x0 && nibbles[3] == 0xA {
                // FX0A - block until get key
                if let Some(blocking_state) = &mut self.blocking {
                    if let Some(key) = blocking_state.compare_and_update(&self.keys) {
                        // A key was released. Store the key in VX.
                        self.set_register(nibbles[1], key as u8)?;
                        self.blocking = None;
                        // Continue execution, program counter is already increased
                    } else {
                        self.program_counter -= 2;
                    }
                } else {
                    // Enter blocking state. Remember the keys which were pressed when we entered.
                    self.blocking = Some(BlockingState::new(&self.keys));
                    // Undo the usual advancement of the program counter. Stay at the current instruction.
                    self.program_counter -= 2;
                }
            }
            if nibbles[2] == 0x2 && nibbles[3] == 0x9 {
                // FX29 - point index register to font character
                let value = self.register(nibbles[1])?;
                if value < 16 {
                    self.index_register = FONT_START_ADDR + value as u16 * 5;
                }
                // No warning is emitted.
            } else if nibbles[2] == 0x3 && nibbles[3] == 0x3 {
                // FX33 - binary-coded decimal conversion
                let value = self.register(nibbles[1])?;
                let digit1 = value / 100;
                self.memory[self.index_register as usize] = digit1;
                let value = value % 100;
                let digit2 = value / 10;
                self.memory[self.index_register as usize + 1] = digit2;
                let value = value % 10;
                let digit3 = value;
                self.memory[self.index_register as usize + 2] = digit3;
            } else if nibbles[2] == 0x5 && nibbles[3] == 0x5 {
                // FX55 - store registers up to VX in memory pointed to by index register
                let max = nibbles[1];
                for i in 0..=max {
                    self.set_memory(self.index_register + i as u16, self.register(i)?)?;
                }
                if self.settings.inc_i_in_fx55_and_fx65 {
                    self.index_register += max as u16 + 1;
                }
            } else if nibbles[2] == 0x6 && nibbles[3] == 0x5 {
                // FX65 - load registers from memory
                let max = nibbles[1];
                for i in 0..=max {
                    self.set_register(i, self.memory(self.index_register + i as u16)?)?;
                }
                if self.settings.inc_i_in_fx55_and_fx65 {
                    self.index_register += max as u16 + 1;
                }
            }
        } else {
            return Err(ExecutionError::UnknownInstruction(instruction));
        }
        Ok(())
    }

    /// Handle the clock signal (60 times per second) by decreasing the delay timer and sound timer
    /// registers.
    pub fn handle_timer_tick(&mut self) {
        if self.delay_timer > 0 {
            self.delay_timer -= 1;
        }
        if self.sound_timer > 0 {
            self.sound_timer -= 1;
        }
    }

    /// Get the value in register `index`.
    fn register(&self, index: u8) -> Result<u8, ExecutionError> {
        self.variable_registers
            .get(index as usize)
            .copied()
            .ok_or(ExecutionError::RegisterIndexOutOfRange(index))
    }

    /// Set the value in register `index`.
    fn set_register(&mut self, index: u8, value: u8) -> Result<(), ExecutionError> {
        *self
            .variable_registers
            .get_mut(index as usize)
            .ok_or(ExecutionError::RegisterIndexOutOfRange(index))? = value;
        Ok(())
    }

    #[allow(dead_code)]
    fn flag_register(&self) -> u8 {
        *self
            .variable_registers
            .get(FLAG_REGISTER_INDEX as usize)
            .expect("const FLAG_REGISTER_INDEX is out of range")
    }

    /// Set the flag register.
    fn set_flag_register(&mut self, value: u8) {
        *self
            .variable_registers
            .get_mut(FLAG_REGISTER_INDEX as usize)
            .expect("const FLAG_REGISTER_INDEX is out of range") = value;
    }

    /// Read the memory cell at `address`.
    fn memory(&self, address: u16) -> Result<u8, ExecutionError> {
        self.memory
            .get(address as usize)
            .copied()
            .ok_or(ExecutionError::MemoryAccessOutOfBounds)
    }

    /// Write to the memory cell at `address`.
    fn set_memory(&mut self, index: u16, value: u8) -> Result<(), ExecutionError> {
        *self
            .memory
            .get_mut(index as usize)
            .ok_or(ExecutionError::MemoryAccessOutOfBounds)? = value;
        Ok(())
    }

    /// Get the current content of the display.
    pub fn display(&self) -> Display {
        Display {
            content: self.display.clone(),
        }
    }

    #[allow(dead_code)]
    pub fn is_blocking(&self) -> bool {
        self.blocking.is_some()
    }

    /// Return true if sound is currently playing.
    pub fn playing_sound(&self) -> bool {
        self.sound_timer > 0
    }

    /// Accept keyboard input.
    pub fn handle_keys(&mut self, keys: HashSet<Key>) {
        self.keys = keys;
    }
}

#[derive(Debug, Clone)]
struct BlockingState {
    keys_on_enter: HashSet<Key>,
}

impl BlockingState {
    fn new(keys: &HashSet<Key>) -> Self {
        BlockingState {
            keys_on_enter: keys.clone(),
        }
    }

    /// Return `Some(k)` if there is at least one new key in `keys`. Which of the new keys in
    /// returned as `k` is random. Return None of there are fewer keys or no change.
    fn compare_and_update(&mut self, keys: &HashSet<Key>) -> Option<Key> {
        let released = self.keys_on_enter.difference(keys).next().cloned();
        self.keys_on_enter = keys.clone();
        released
    }
}

#[derive(Debug, Error)]
pub enum EmulatorError {
    #[error(transparent)]
    Loading(#[from] LoadingError),

    #[error("invalid instruction ({instruction:#06x} at {address:#06x}): {source}")]
    Execution {
        address: u16,
        instruction: u16,
        source: ExecutionError,
    },
}

#[derive(Error, Debug)]
pub enum LoadingError {
    #[error("ROM too large")]
    RomTooLarge,
}

#[derive(Debug, Error)]
pub enum ExecutionError {
    #[error("stack underflow")]
    StackUnderflow,

    #[error("register index '{0}' out of range")]
    RegisterIndexOutOfRange(u8),

    #[error("memory access out of bounds")]
    MemoryAccessOutOfBounds,

    #[error("unknown instruction '{0:#06x}'")]
    UnknownInstruction(u16),
}

#[derive(Clone, Debug, Default)]
pub struct Display {
    pub content: Vec<bool>,
}

impl Display {
    pub fn get(&self, x: u8, y: u8) -> bool {
        // If the display is empty (content vector has length zero), return black, i.e. false
        let index = y as usize * DISPLAY_WIDTH as usize + x as usize;
        self.content.get(index).copied().unwrap_or(false)
    }
}

/// Get the nibble (half-byte) at `index` of `value`.
///
/// # Panics
///
/// Panics if `index` is greater than 3.
fn nibble(value: u16, index: u8) -> u8 {
    match index {
        0 => (value >> 12 & 0x000f) as u8,
        1 => (value >> 8 & 0x000f) as u8,
        2 => (value >> 4 & 0x000f) as u8,
        3 => (value & 0x000f) as u8,
        _ => panic!("invalid nibble index"),
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct InstructionSettings {
    use_vy_in_8xy6: bool,
    use_vy_in_8xye: bool,
    use_bxnn_instead_bnnn: bool,
    set_vf_on_overflow_in_fx1e: bool,
    inc_i_in_fx55_and_fx65: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for InstructionSettings {
    fn default() -> Self {
        InstructionSettings {
            use_vy_in_8xy6: false,
            use_vy_in_8xye: false,
            use_bxnn_instead_bnnn: false,
            set_vf_on_overflow_in_fx1e: false,
            inc_i_in_fx55_and_fx65: false,
        }
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Num0 = 0x0,
    Num1 = 0x1,
    Num2 = 0x2,
    Num3 = 0x3,
    Num4 = 0x4,
    Num5 = 0x5,
    Num6 = 0x6,
    Num7 = 0x7,
    Num8 = 0x8,
    Num9 = 0x9,
    A = 0xA,
    B = 0xB,
    C = 0xC,
    D = 0xD,
    E = 0xE,
    F = 0xF,
}

impl TryFrom<u8> for Key {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(Key::Num0),
            0x1 => Ok(Key::Num1),
            0x2 => Ok(Key::Num2),
            0x3 => Ok(Key::Num3),
            0x4 => Ok(Key::Num4),
            0x5 => Ok(Key::Num5),
            0x6 => Ok(Key::Num6),
            0x7 => Ok(Key::Num7),
            0x8 => Ok(Key::Num8),
            0x9 => Ok(Key::Num9),
            0xA => Ok(Key::A),
            0xB => Ok(Key::B),
            0xC => Ok(Key::C),
            0xD => Ok(Key::D),
            0xE => Ok(Key::E),
            0xF => Ok(Key::F),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nibble() {
        assert_eq!(nibble(0x1234, 0), 1);
        assert_eq!(nibble(0x1234, 1), 2);
        assert_eq!(nibble(0x1234, 2), 3);
        assert_eq!(nibble(0x1234, 3), 4);
    }

    #[test]
    #[should_panic]
    fn test_nibble_invalid_index() {
        nibble(0x1234, 4);
    }
}
