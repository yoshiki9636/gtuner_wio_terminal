/*
 * Guiter tuner for wio-terminal with Rust
 * Main file
 *
 * Rust 2018 0.1.0
 *
 * @auther     Yoshiki Kurokawa <yoshiki.963@gmail.com>
 * @copylight  2021 Yoshiki Kurokawa
 * @license    https://opensource.org/licenses/MIT     MIT license
 * @version    0.2  2nd version. All function implemented
 * @version    0.1  1st version. Only frequency meter implemented
 */

#![no_std]
#![no_main]

use panic_halt as _;
use wio_terminal as wio;

use core::fmt::Write;
use cortex_m::peripheral::NVIC;
use heapless::consts::*;
use heapless::Vec;
use heapless::String;
use libm::*;
use wio::{entry, Pins, Sets};
use wio::hal::adc::{FreeRunning, InterruptAdc};
use wio::hal::clock::GenericClockController;
use wio::hal::delay::Delay;
use wio::hal::gpio::*;
use wio::pac::{interrupt, CorePeripherals, Peripherals, ADC1};
use wio::prelude::*;
use eg::{
    egrectangle, egtext, fonts::Font24x32, pixelcolor::Rgb565,
    prelude::*, primitive_style, text_style,
};
use embedded_graphics as eg;

// Buffer structure
struct Ctx {
    adc: InterruptAdc<ADC1, FreeRunning>,
    buffers: [SamplingBuffer; 2],
    sampling_buffer: Option<&'static mut SamplingBuffer>,
    processing_buffer: Option<&'static mut SamplingBuffer>,
}

static mut CTX: Option<Ctx> = None;

const SMP_POINTS: usize = 2048;
const ADC_SAMPLING_RATE: f32 = 83333.0;
const ADC_SAMPLING_ADJ:  f32 = 356.0;
const SMP_THRE: u16 = 2048;

const SCREEN_WIDTH: i32 = 320;
const SCREEN_HEIGHT: i32 = 240;
const FONT_WIDTH: i32 = 24;
const FONT_HEIGHT: i32 = 32;

type SamplingBuffer = heapless::Vec<u16, U2048>;

#[entry]
fn main() -> ! {

    // Initializing peripherals
    let mut peripherals = Peripherals::take().unwrap();
    let core = CorePeripherals::take().unwrap();
    let mut clocks = GenericClockController::with_external_32kosc(
        peripherals.GCLK,
        &mut peripherals.MCLK,
        &mut peripherals.OSC32KCTRL,
        &mut peripherals.OSCCTRL,
        &mut peripherals.NVMCTRL,
    );
    
    // GPIO initialization
    // using button3 for up frequency, button2 for down frequency
    let mut sets: Sets = Pins::new(peripherals.PORT).split();
    let button_up   = sets.buttons.button3.into_floating_input(&mut sets.port);
    let button_down = sets.buttons.button2.into_floating_input(&mut sets.port);

    // UART initialization for debugging monitor
    let mut delay = Delay::new(core.SYST, &mut clocks);

    let mut serial = sets.uart.init(
        &mut clocks,
        115200.hz(),
        peripherals.SERCOM2,
        &mut peripherals.MCLK,
        &mut sets.port,
    );

    // Microphone ADC initialization
    let (microphone_adc, mut microphone_pin) = sets.microphone.init(
        peripherals.ADC1,
        &mut clocks,
        &mut peripherals.MCLK,
        &mut sets.port,
    );
    let mut microphone_adc: InterruptAdc<_, FreeRunning> = InterruptAdc::from(microphone_adc);
    // start ADC
    microphone_adc.start_conversion(&mut microphone_pin);

    // LCD initialization
    let (mut display, _backlight) = sets
        .display
        .init(
            &mut clocks,
            peripherals.SERCOM7,
            &mut peripherals.MCLK,
            &mut sets.port,
            60.mhz(),
            &mut delay,
        )
        .unwrap();

    // Draw black to LCD
    egrectangle!(
        top_left = (0, 0),
        bottom_right = (SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1),
        style = primitive_style!(fill_color = Rgb565::BLACK)
    )
    .draw(&mut display)
    .unwrap();
    // Draw tuner mater center marker
    egrectangle!(
        top_left = (SCREEN_WIDTH/2-5,0),
        bottom_right = (SCREEN_WIDTH/2+5, 32),
        style = primitive_style!(fill_color = Rgb565::BLUE)
    )
    .draw(&mut display)
    .unwrap();
    // draw inital values
    draw_freq(&mut display, 0.0, 0);
    draw_freq(&mut display, 440.0, 1);

    // shared resources initialization
    unsafe {
        CTX = Some(Ctx {
            adc: microphone_adc,
            buffers: [Vec::new(), Vec::new()],
            sampling_buffer: None,
            processing_buffer: None,
        });
        // set buffers
        let mut ctx = CTX.as_mut().unwrap();
        let (first, rest) = ctx.buffers.split_first_mut().unwrap();
        ctx.sampling_buffer = Some(first);
        ctx.processing_buffer = Some(&mut rest[0]);
    }
    writeln!(&mut serial, "start").unwrap(); // for debugging
    unsafe { NVIC::unmask(interrupt::ADC1_RESRDY); }
    writeln!(&mut serial, "check1").unwrap(); // for debugging

    // Pitch frequency initialization
    let mut fpitch = 440.0 as f32;
    // buttons press status
    let mut press_up   = 0 as u8;
    let mut press_down = 0 as u8;
    // Loop variable initialization
    let mut cntr = 0; // loop counter
    let mut asum = 0; // sum of all data for average to get frecuency
    let mut anum = 0; // number of all data for average to get frecuency
    // main loop
    loop {
        // get processing buffer 
        let processing_buffer = unsafe {
            let ctx = CTX.as_mut().unwrap();
            ctx.processing_buffer.as_mut().unwrap()
        };
        let len = processing_buffer.len();
        let cap = processing_buffer.capacity();
        // if processing buffer is full -> processing
        //                     not full -> skip processing and continue to next loop
        if len == cap {
            // temporary buffers for filterling
            let mut oarray: [u16; SMP_POINTS] = [0;SMP_POINTS];
            let mut barray: [u8; SMP_POINTS] = [0;SMP_POINTS];
            // normalization filter :: output: oarray, avg, flg
            let (avg, flg) = normalization(&processing_buffer, &mut oarray);
            // attack cancelletion filter : threshold : SMP_THRE
            if flg == 1 {
                processing_buffer.clear();
                continue;
            }
            // low pass filter & make barray data
            // 1. 7 tap moving average for low pass filter
            // 2. make barray data : binary data : AVG> -> 0  AVG<= -> 1
            get_barray(&oarray, avg, &mut barray);
            // inflate filter
            //   infrate 1 data of edge (0-1, 1-0 or 1-0, 0-1 : depend on num)
            // Inflate 1 : 2 times
            inflate_barray(&mut barray, 1);
            inflate_barray(&mut barray, 1);
            // Inflate 0 : 4 times
            inflate_barray(&mut barray, 0);
            inflate_barray(&mut barray, 0);
            inflate_barray(&mut barray, 0);
            inflate_barray(&mut barray, 0);
            // Inflate 1 : 2 times
            inflate_barray(&mut barray, 1);
            inflate_barray(&mut barray, 1);
            // get rise edge period, count, max-min
            let (psum, pnum, pdiff) = get_rise_edge(&barray); 

            /*
            // for debugging
            // dump data
            for i in 0..SMP_POINTS {
                //write!(&mut serial, "{} ",processing_buffer[i]).unwrap();
                write!(&mut serial, "{}",barray[i]).unwrap();
                if i % 16 == 15 {
                    nb::block!(serial.write(0x0d as u8)).unwrap();
                }
            }
            nb::block!(serial.write(0x0d as u8)).unwrap();
            */
            // pdiff : fructuation of piriods : if it is large, average has no meaning.
            //         if it is large, do nothing and clear buffer
            if pdiff <= 100 {
                // add to loop variable
                asum += psum;
                anum += pnum;
                cntr += 1;
                // 20 data are used for frecuency result
                if cntr == 20 {
                    // calcurate frecuency
                    //   ADC_SAMPLING_ADJ is sampling rate adust parameter
                    let favg = asum as f32 / anum as f32;
                    let freq = (ADC_SAMPLING_RATE - ADC_SAMPLING_ADJ) / favg;
                    writeln!(&mut serial, "avg: {}",favg).unwrap(); // for debugging
                    writeln!(&mut serial, "frequency: {}",freq).unwrap(); // for debugging
                    // clear loop parameter
                    asum = 0;
                    anum = 0;
                    cntr = 0;

                    // get note number and fine difference from frequency
                    let (note,diff) = get_note_from_freq(fpitch, freq);
                    // draw fine meter
                    draw_meter(&mut display, diff);
                    // draw note name and number
                    draw_note(&mut display, note);
                    // draw wave frequeancy
                    draw_freq(&mut display, freq, 0);
                    // draw pitch frequency
                    draw_freq(&mut display, fpitch, 1);
                    /*
                    // for debugging
                    // waiting 1 charactor for the next
                    loop {
                        if let Ok(c) = nb::block!(serial.read()) {
                            nb::block!(serial.write(c)).unwrap();
                            break;
                        }
                    }
                    */
                }
            }
            processing_buffer.clear();
        }
        /*
        // for debugging
        // waiting 1 charactor for the next
        loop {
            if let Ok(c) = nb::block!(serial.read()) {
                nb::block!(serial.write(c)).unwrap();
                break;
            }
        }
        */
        // scanning buttons
        if (button_up.is_low().unwrap())&(press_up == 0) {
            fpitch += 1.0 as f32;
            press_up = 1
        } else if button_up.is_high().unwrap() {
            press_up = 0
        }
        if (button_down.is_low().unwrap())&(press_down == 0) {
            fpitch -= 1.0 as f32;
            press_down = 1
        } else if button_down.is_high().unwrap() {
            press_down = 0
        }
    }
}

// normalization function
fn normalization(iarray: &[u16], oarray: &mut [u16]) -> (f32, u32)  {
    // get min,max
    let mut min = 0x7fff as u16;
    let mut max = 0 as u16;
    for i in 0..SMP_POINTS-1 {
        if iarray[i] > max {
            max = iarray[i];
        }
        else if iarray[i] < min {
            min = iarray[i];
        }
    }
    // get intensity flg 0: OK 1:NG
    let flg = if max - min > SMP_THRE { 1 } else { 0 };

    // get normalized data -> oarray
    let mut sum = 0.0 as u32;
    for i in 0..SMP_POINTS-1 {
        // scaling
        let tmp = ((iarray[i] - min) as f32 * (0x00003fff as f32/(max - min) as f32)) as u16;
        oarray[i] = tmp;
        // sum for average
        sum += tmp as u32;
    }
    (sum as f32 / SMP_POINTS as f32, flg)
}

// moving average and making binary array
fn get_barray(iarray: &[u16], avg: f32, barray: &mut [u8]) {
    // first and last 3 result does not exist becase these positions cannot get 7 taps. 
    for i in 3..SMP_POINTS-3 {
        // moving average
        let sum = iarray[i-3] as u32 + iarray[i-2] as u32 + iarray[i-1] as u32 + iarray[i] as u32
                              + iarray[i+1] as u32 + iarray[i+2] as u32 + iarray[i+3] as u32;
        let diff = sum as f32 / 7.0 - avg;
        // making binary data
        if diff < 0.0 {
            barray[i] = 0;
        } else {
            barray[i] = 1;
        }
    }
}

// infragte binary data
//   num : choosing infrate data, 0 or 1
fn inflate_barray(barray: &mut [u8], num: u8) {
    // copy data to temporary buffer carray
    let mut carray: [u8; SMP_POINTS] = [0;SMP_POINTS];
    for i in 3..SMP_POINTS-3 {
        carray[i] = barray[i];
    }
    // infrate function
    //   carray : reference data 
    //   barray : modifying data 
    for i in 3..SMP_POINTS-4 {
        if (carray[i] == num)&(carray[i+1] != num) {
            barray[i+1] = num;
        }
        else if (carray[i] != num)&(carray[i+1] == num) {
            barray[i] = num;
        }
    }
}

// getting rise edge (0->1) to get piriod of wave
fn get_rise_edge(barray: &[u8]) -> (u32, u32, u32) {
    // Getting rise edge positions and difference of positons that is wave periods.
    // Last data of barray does not use because the data is meaningless.
    // Term list has perods of waves.
    let mut term: heapless::Vec<u16,U50> = Vec::new();
    let mut current = 0;
    for i in 3..SMP_POINTS-5 {
        if (barray[i] == 0)&(barray[i+1] == 1) {
            if current == 0 {
                current = i;
            } else {
                term.push((i - current) as u16);
                current = i;
            }
        }
    }
    // getting sum of wave periods, count, max-min(diff)
    let mut sum = 0;
    let mut num = 0;
    let mut min = 10000;
    let mut max = 0;
    for d in &term {
        sum += *d as u32;
        num += 1;
        if *d > max {
            max = *d;
        }
        if *d < min {
            min = *d;
        }
    }
    let diff = max - min;
    (sum, num, diff as u32)
}

// getting note name
//  Input note is a number of note which starts A/0 as 1
fn get_note_name(note: u32) -> (&'static str, u32) {
    let num  = (note + 8) / 12;
    let tone = (note + 8) % 12;
    let mut name = "";
    match tone {
        0 => name = "C",
        1 => name = "C#",
        2 => name = "D",
        3 => name = "D#",
        4 => name = "E",
        5 => name = "F",
        6 => name = "F#",
        7 => name = "G",
        8 => name = "G#",
        9 => name = "A",
        10 => name = "A#",
        11 => name = "B",
        n => name = "XX",
    }
    return (&name,num)
}

// Transrate note number from frequency using equal temperament
//  Equal temperament : f(k) = f(1)*2^(k/12)
//                          k = i - 49  (means f(1)=A/4=440Hz)
//  This function is inverse funciton of it.
//    i = 49 + 12 * log2(f(k)/f(1))
//   tone is an integer part of i
//   diff is a fraction part of i and scaled to display size
fn get_note_from_freq(fpitch: f32, freq: f32) -> (u32, f32) {
    let tone_num = 49.0 + 12.0 * log2f(freq / fpitch);
    let tone = (tone_num + 0.5) as u32;
    let diff = (tone_num - (tone as f32)) * (SCREEN_WIDTH as f32 - 20.0)
                + SCREEN_WIDTH as f32 / 2.0;
    return (tone,diff);
}

// draw freqrency part
fn draw_freq<T>(display: &mut T, freq: f32, flag: u8)
where
    T: embedded_graphics::DrawTarget<Rgb565>,
{
    let y = if flag == 0 {150} else {192};
    // clear area
    egrectangle!(
        top_left = (0,y),
        bottom_right = (SCREEN_WIDTH-1, y+FONT_HEIGHT),
        style = primitive_style!(fill_color = Rgb565::BLACK)
    )
    .draw(display);

    // draw frequency
    let mut textbuffer = String::<U256>::new();
    if flag == 0 {
        write!(&mut textbuffer, "{:.2} Hz", freq).unwrap();
    } else {
        write!(&mut textbuffer, "P {:.0} Hz", freq).unwrap();
    }
    //
    let length = textbuffer.len();
    //
    let left = SCREEN_WIDTH - (length as i32) * FONT_WIDTH;
    egtext!(
        text = textbuffer.as_str(),
        top_left = (left, y),
        style = text_style!(font = Font24x32, text_color = Rgb565::WHITE)
    )
    .draw(display);
}

// draw note part
fn draw_note<T>(display: &mut T, note: u32)
where
    T: embedded_graphics::DrawTarget<Rgb565>,
{
    // clear area
    let y = 108;
    egrectangle!(
        top_left = (0,y),
        bottom_right = (SCREEN_WIDTH-1, y+FONT_HEIGHT),
        style = primitive_style!(fill_color = Rgb565::BLACK)
    )
    .draw(display);

    // get note name form note num
    let (name,num) = get_note_name(note);

    let mut textbuffer = String::<U256>::new();
    write!(&mut textbuffer, "{} / {}", name, num).unwrap();
    //
    let length = textbuffer.len();
    //
    let left = SCREEN_WIDTH - (length as i32) * FONT_WIDTH;
    egtext!(
        text = textbuffer.as_str(),
        top_left = (left, y),
        style = text_style!(font = Font24x32, text_color = Rgb565::WHITE)
    )
    .draw(display);
}

// draw meter part
fn draw_meter<T>(display: &mut T, diff: f32)
where
    T: embedded_graphics::DrawTarget<Rgb565>,
{
    // clear area
    egrectangle!(
        top_left = (0,32),
        bottom_right = (SCREEN_WIDTH-1, 64),
        style = primitive_style!(fill_color = Rgb565::BLACK)
    )
    .draw(display);

    egrectangle!(
        top_left = (diff as i32,32),
        bottom_right = (diff as i32 + 10, 64),
        style = primitive_style!(fill_color = Rgb565::WHITE)
    )
    .draw(display);
    /*
    // for debuffing
    let mut textbuffer = String::<U256>::new();
    write!(&mut textbuffer, "{}", diff).unwrap();
    let length = textbuffer.len();
    let left = SCREEN_WIDTH - (length as i32) * FONT_WIDTH;
    egtext!(
        text = textbuffer.as_str(),
        top_left = (left, 32),
        style = text_style!(font = Font24x32, text_color = Rgb565::WHITE)
    )
    .draw(display);
    */
}

// interrupt part
//   Currently if sampling buffer is full when processing buffer still used,
//   the data is thrown away and clear sampling buffer, because of processing latest data.
#[interrupt]
fn ADC1_RESRDY() {
    unsafe {
        let ctx = CTX.as_mut().unwrap();
        if let Some(sample) = ctx.adc.service_interrupt_ready() {
            // data is in sample
            let sampling_buffer = ctx.sampling_buffer.as_mut().unwrap();
            if sampling_buffer.len() == sampling_buffer.capacity() {
                // sampling buffer full
                if ctx.processing_buffer.as_mut().unwrap().len() == 0 {
                    core::mem::swap(
                        &mut ctx.processing_buffer,
                        &mut ctx.sampling_buffer,
                    );
                } else {
                    sampling_buffer.clear(); // throw away data and restart sampling
                }
            } else {
                let _ = sampling_buffer.push(sample);
            }
        }
    }
}
