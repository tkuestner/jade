use std::collections::HashSet;
use std::sync::mpsc;

use log::{error, trace, warn};

pub use crate::processor::{Display, InstructionSettings, Key, DISPLAY_HEIGHT, DISPLAY_WIDTH};
use crate::processor::{EmulatorError, Processor};
use crate::sound::Sound;

const TIMER_INTERVAL: std::time::Duration = std::time::Duration::from_micros(16666);
const DEFAULT_INSTRUCTIONS_PER_SECOND: usize = 700;

/// The main part of the CHIP-8 emulator. Uses threading internally.
pub struct Emulator {
    sender: mpsc::Sender<Request>,
    receiver: mpsc::Receiver<Response>,
    #[allow(dead_code)]
    timer: timer::Timer,
    #[allow(dead_code)]
    guard: timer::Guard,
}

impl Emulator {
    /// Start a new emulator in a separate thread. This function also sets up the required
    /// timers (delay and sound timer).
    pub fn new() -> Self {
        // Channel from the emulator (handle) to the executor
        let (sender, executor_receiver) = mpsc::channel();
        // Channel from the executor back to the emulator
        let (executor_sender, receiver) = mpsc::channel();

        let s = sender.clone();
        let timer = timer::Timer::new();
        let duration =
            chrono::TimeDelta::from_std(TIMER_INTERVAL).expect("timer duration out of range");
        let guard = timer.schedule_repeating(duration, move || {
            let _ = s.send(Request::TimerTick);
        });

        trace!("starting emulator");
        std::thread::spawn(move || {
            let mut emulator = Executor::new(executor_receiver, executor_sender);
            emulator.start()
        });

        // Note the thread is detached deliberately. When Emulator is dropped, the request
        // channel is closed which in turn triggers the emulator thread to finish executing.
        // We could store the join handle for joining later, but this brings about several problems.
        // (i) Forgetting to call join explicitly. (ii) Implementing drop() and calling join there.
        // (iii) Doing the option dance in drop() so not to join an  already joined thread.

        Self {
            sender,
            receiver,
            timer,
            guard,
        }
    }

    /// Get all responses currently available from previously posted requests.
    pub fn responses(&mut self) -> Vec<Response> {
        let mut responses = Vec::new();
        loop {
            let result = self.receiver.try_recv();
            match result {
                Ok(response) => responses.push(response),
                Err(mpsc::TryRecvError::Empty) => {
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    error!("emulator disconnected");
                    break;
                }
            }
        }
        responses
    }

    /// Request to load settings regarding the instruction set into the emulator.
    pub fn load_settings(&self, settings: InstructionSettings) {
        let msg = Request::LoadSettings(settings);
        self.sender
            .send(msg)
            .expect("EmulatorHandle::load_settings failed to send. Emulator no longer running?");
    }

    /// Request to load a program data (a 'ROM') into the emulator.
    pub fn load_program(&self, data: Vec<u8>) {
        let msg = Request::LoadProgram(data.to_vec());
        self.sender
            .send(msg)
            .expect("EmulatorHandle::load_program failed. Emulator no longer running?");
    }

    /// Request to start running the program.
    pub fn run_program(&self, speed: Speed) {
        let msg = Request::RunProgram(speed);
        self.sender
            .send(msg)
            .expect("EmulatorHandle::run_program failed to send. Emulator no longer running?");
    }

    /// Request to stop running the program.
    pub fn stop(&self) {
        let msg = Request::Stop;
        self.sender
            .send(msg)
            .expect("EmulatorHandle::stop failed. Emulator no longer running?");
    }

    /// Request to step the program by one instruction.
    pub fn step(&self) {
        let msg = Request::Step;
        self.sender
            .send(msg)
            .expect("EmulatorHandle::step failed. Emulator no longer running?");
    }

    /// Request the display data.
    pub fn query_display(&self) {
        let msg = Request::Display;
        self.sender
            .send(msg)
            .expect("EmulatorHandle::display failed. Emulator no longer running?");
    }

    /// Request the current program state.
    pub fn query_state(&self) {
        let msg = Request::State;
        self.sender
            .send(msg)
            .expect("EmulatorHandle::state failed. Emulator no longer running?");
    }

    /// Pass the currently pressed keys to the emulator.
    pub fn send_keys(&self, keys: &HashSet<Key>) {
        let msg = Request::SendKeys(keys.clone());
        self.sender
            .send(msg)
            .expect("EmulatorHandle::send failed. Emulator no longer running?");
    }
}

impl Default for Emulator {
    fn default() -> Self {
        Self::new()
    }
}

/// List of requests which can used by the client (UI) to control the emulator.
#[derive(Debug)]
enum Request {
    LoadSettings(InstructionSettings),
    LoadProgram(Vec<u8>),
    RunProgram(Speed),
    Stop,
    Step,
    Display,
    State,
    SendKeys(HashSet<Key>),
    TimerTick,
}

/// List of responses sent from emulator as an answer to a client request. Not all requests have
/// corresponding responses.
#[derive(Debug)]
pub enum Response {
    LoadProgram(Result<(), EmulatorError>),
    Step(Result<(), EmulatorError>),
    Display(Display),
    State(ProgramState),
    RunError(EmulatorError),
}

/// Executor part of the emulator. Receives client requests, contains the core loop and handles
/// things like execution speed and sound.
struct Executor {
    receiver: mpsc::Receiver<Request>,
    sender: mpsc::Sender<Response>,
    emulator: Processor,
    state: ProgramState,
    sound: Option<Sound>,
    speed: Speed,
    instruction_account_balance: usize,
}

impl Executor {
    /// Create a new executor capable of handling requests and sending responses. This function
    /// also initialized the sound system.
    fn new(receiver: mpsc::Receiver<Request>, sender: mpsc::Sender<Response>) -> Self {
        let sound = Sound::new();
        if let Err(e) = &sound {
            warn!("failed to initialize sound: {}", e);
        }
        let sound = sound.ok();

        Executor {
            receiver,
            sender,
            emulator: Processor::new(),
            state: ProgramState::Stopped,
            sound,
            speed: Speed(DEFAULT_INSTRUCTIONS_PER_SECOND),
            instruction_account_balance: 0,
        }
    }

    /// Start running the executor. This function contains the core loop which waits for
    /// requests from the client.
    fn start(&mut self) {
        loop {
            let request = if let Ok(request) = self.receiver.recv() {
                request
            } else {
                trace!("emulator exiting because request channel was closed");
                break;
            };

            self.handle(request);

            while self.instruction_account_balance > 0 {
                match self.emulator.step() {
                    Ok(_) => {
                        self.instruction_account_balance -= 1;
                    }
                    Err(e) => {
                        self.state = ProgramState::Stopped;
                        self.instruction_account_balance = 0;
                        let _ = self.sender.send(Response::RunError(e));
                    }
                }
            }
            self.handle_sound();
        }
        trace!("emulator finished running");
    }

    /// Dispatch and handle client requests.
    fn handle(&mut self, msg: Request) {
        match msg {
            Request::LoadSettings(settings) => {
                self.emulator.load_settings(settings);
            }
            Request::LoadProgram(data) => {
                let result = self.emulator.load_program(data);
                let _ = self.sender.send(Response::LoadProgram(result));
            }
            Request::Step => {
                let result = self.emulator.step();
                self.handle_sound();
                let _ = self.sender.send(Response::Step(result));
            }
            Request::RunProgram(speed) => {
                self.speed = speed;
                if self.state != ProgramState::Running {
                    self.state = ProgramState::Running;
                }
            }
            Request::Display => {
                let display = self.emulator.display();
                let _ = self.sender.send(Response::Display(display));
            }
            Request::State => {
                let _ = self.sender.send(Response::State(self.state));
            }
            Request::Stop => {
                self.state = ProgramState::Stopped;
            }
            Request::TimerTick => {
                if self.state == ProgramState::Running {
                    let Speed(instructions_per_second) = self.speed;
                    let tick_interval = TIMER_INTERVAL.as_secs_f32();
                    let instructions_per_tick = instructions_per_second as f32 * tick_interval;
                    self.instruction_account_balance = instructions_per_tick.ceil() as usize;
                }
                self.emulator.handle_timer_tick();
            }
            Request::SendKeys(keys) => {
                self.emulator.handle_keys(keys);
            }
        }
    }

    /// Handle sound.
    fn handle_sound(&mut self) {
        if let Some(sound) = &mut self.sound {
            if self.emulator.playing_sound() {
                sound.play();
            } else {
                sound.pause();
            }
        }
    }
}

/// Program execution speed. Instructions per second.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Speed(usize);

impl Speed {
    pub fn new(instructions_per_second: usize) -> Self {
        Speed(instructions_per_second)
    }
}

/// The program state. Running or stopped.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum ProgramState {
    Running,
    Stopped,
}
