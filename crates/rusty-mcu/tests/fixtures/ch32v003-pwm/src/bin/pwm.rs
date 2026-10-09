#![no_std]
#![no_main]

use hal::delay::Delay;
use hal::time::Hertz;
use hal::timer::low_level::CountingMode;
use hal::timer::simple_pwm::{PwmPin, SimplePwm};
use {ch32_hal as hal, panic_halt as _};

#[qingke_rt::entry]
fn main() -> ! {
    hal::debug::SDIPrint::enable();
    let p = hal::init(Default::default());

    let pin = PwmPin::new_ch4::<0>(p.PC4);
    let mut pwm = SimplePwm::new(
        p.TIM1,
        None,
        None,
        None,
        Some(pin),
        Hertz::khz(1),
        CountingMode::default(),
    );
    let ch = hal::timer::Channel::Ch4;

    let max = pwm.get_max_duty();
    hal::println!("pwm max duty {}", max);
    pwm.set_duty(ch, 0);
    pwm.enable(ch);

    loop {
        for i in 0..100 {
            pwm.set_duty(ch, max * i / 100);
            Delay.delay_ms(10);
        }
        hal::println!("up");
        for i in (0..100).rev() {
            pwm.set_duty(ch, max * i / 100);
            Delay.delay_ms(10);
        }
        hal::println!("down");
    }
}
