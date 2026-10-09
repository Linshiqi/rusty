//! ch32-hal's PWM example for the CH32X035, without embassy: TIM1 channel 4
//! on PB12 at 1 kHz, the duty walked up and down a hundredth every 10 ms,
//! a button on PB1 that holds it at full, and SDI print saying where it is.
//! A multiply and a divide in the loop, because the V4C has the M extension
//! and the emulator must give the same answers the part does.

#![no_std]
#![no_main]

use hal::delay::Delay;
use hal::gpio::{Input, Pull};
use hal::time::Hertz;
use hal::timer::low_level::CountingMode;
use hal::timer::simple_pwm::{PwmPin, SimplePwm};
use {ch32_hal as hal, panic_halt as _};

#[qingke_rt::entry]
fn main() -> ! {
    hal::debug::SDIPrint::enable();
    let p = hal::init(Default::default());

    let pin = PwmPin::new_ch4::<2>(p.PB12);
    let mut pwm = SimplePwm::new(
        p.TIM1,
        None,
        None,
        None,
        Some(pin),
        Hertz::khz(1),
        CountingMode::default(),
    );
    let channel = hal::timer::Channel::Ch4;
    let full = pwm.get_max_duty();
    pwm.enable(channel);
    let button = Input::new(p.PB1, Pull::Up);

    hal::println!("pwm max duty {}", full);

    let mut level: u32 = 0;
    let mut rising = true;
    loop {
        let duty = if button.is_low() { full } else { full * level / 100 };
        pwm.set_duty(channel, duty);
        if level % 25 == 0 {
            hal::println!("level {} duty {}", level, duty);
        }
        if rising {
            level += 1;
            rising = level < 100;
        } else {
            level -= 1;
            rising = level == 0;
        }
        Delay.delay_ms(10);
    }
}
