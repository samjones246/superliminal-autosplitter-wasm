use spinning_top::{const_spinlock, Spinlock};

use bytemuck::Pod;

use asr::{
    time::Duration,
    timer::{self, TimerState},
    watcher::Pair,
    Address, Process,
};

static STATE: Spinlock<State> = const_spinlock(State { 
    game: None,
});

struct Watcher<T> {
    watcher: asr::watcher::Watcher<T>,
    address: Vec<u64>,
}

impl<T: Pod> Watcher<T> {
    fn new(address: Vec<u64>) -> Self {
        Self {
            watcher: asr::watcher::Watcher::new(),
            address,
        }
    }

    fn update(&mut self, process: &Process, module: u64) -> Option<&Pair<T>> {
        let value = process.read_pointer_path64::<T>(module, &self.address);
        self.watcher.update(value.ok())
    }
}

struct Game {
    process: Process,
    module: u64,
    game_time: Watcher<f64>,
    scene_ptr: Watcher<u64>,
    scene: Pair<String>,
    retro_alarm_clicked: Watcher<u8>,
}

impl Game {
    fn new(process: Process, module: u64) -> Option<Self> {
        let game = Self {
            process,
            module,
            game_time: Watcher::new(vec![0x0195D848, 0x08, 0xB0, 0xC0, 0x28, 0x130]),
            scene_ptr: Watcher::new(vec![0x019151F8, 0x48, 0x10]),
            scene: Pair { old: String::new(), current: String::new() },
            retro_alarm_clicked: Watcher::new(vec![0x0195D848, 0x08, 0xB0, 0xA8, 0x28, 0x141]),
        };
        Some(game)
    }

    fn update_vars(&mut self) -> Option<Vars<'_>> {
        let scene_ptr = self.scene_ptr.update(&self.process, self.module)?;
        let mut buf= [0; 255];
        self.process.read_into_buf(Address(scene_ptr.current), &mut buf).ok()?;
        let value = bytes_to_string(&buf).unwrap_or(String::from("null"));
        self.scene = Pair {old: self.scene.current.clone(), current: value};
        Some(Vars {
            game_time: self.game_time.update(&self.process, self.module)?,
            scene: &self.scene,
            retro_alarm_clicked: self.retro_alarm_clicked.update(&self.process, self.module)?,
        })
    }
}

pub fn bytes_to_string(utf8_src: &[u8]) -> Result<String, std::string::FromUtf8Error> {
    let nul_range_end = utf8_src.iter()
        .position(|&c| c == b'\0')
        .unwrap_or(utf8_src.len());
    String::from_utf8(utf8_src[0..nul_range_end].to_vec())
}

pub struct State {
    game: Option<Game>,
}

struct Vars<'a> {
    game_time: &'a Pair<f64>,
    scene: &'a Pair<String>,
    retro_alarm_clicked: &'a Pair<u8>,
}

#[no_mangle]
pub extern "C" fn update() {
    let mut state = STATE.lock();
    if state.game.is_none() {
        match Process::attach("SuperliminalSteam") {
            Some(process) => {
                match process.get_module_address("UnityPlayer.dylib") {
                    Ok(Address(module)) => {
                        state.game = Game::new(process, module);
                        asr::print_message("attached to process successfully");
                    },
                    _ => {
                        asr::print_message("module not found")
                    },
                };
            }
            None => (),
        }
    }
    if let Some(game) = &mut state.game {
        if !game.process.is_open() {
            state.game = None;
            return;
        }
        if let Some(vars) = &mut game.update_vars() {
            match timer::state() {
                TimerState::NotRunning => {
                    // -- Start
                    if vars.game_time.current > 0.0 && vars.game_time.current != vars.game_time.old {
                        asr::print_message("start");
                        timer::start();
                    }
                }
                TimerState::Running => {
                    timer::set_game_time(Duration::seconds_f64(vars.game_time.current));

                    if vars.scene.changed() {
                        asr::print_message(&vars.scene.current);
                        if vars.scene.starts_with("Assets/_Levels/_LiveFolder/Misc/LoadingScenes/") &&
                           vars.scene.old.starts_with("Assets/_Levels/_LiveFolder/ACT")
                        {
                            timer::split();
                        }

                        else if vars.scene.ends_with("StartScreen_Live.unity") {
                            timer::reset();
                        }
                    }

                    if vars.scene.ends_with("TestChamber_Live.unity") && vars.game_time.decreased(){
                        timer::reset();
                    }

                    if vars.scene.ends_with("EndingMontage_Live.unity") && 
                       vars.retro_alarm_clicked.changed_from_to(&0, &1) 
                    {
                        timer::split();
                    }
                }
                _ => {}
            }
        }

    }
}