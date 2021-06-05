/*
 * Guiter tuner for wio-terminal with Rust
 * Main file
 *
 * Rust 2018 0.1.0
 *
 * @auther     Yoshiki Kurokawa <yoshiki.963@gmail.com>
 * @copylight  2021 Yoshiki Kurokawa
 * @license    https://opensource.org/licenses/MIT     MIT license
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
use wio::pac::{interrupt, CorePeripherals, Peripherals, ADC1};
use wio::prelude::*;
use eg::{
    egrectangle, egtext, fonts::Font24x32, pixelcolor::Rgb565,
    prelude::*, primitive_style, text_style,
};
use embedded_graphics as eg;

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
const SMP_THRE: u16 = 1024;

type SamplingBuffer = heapless::Vec<u16, U2048>; //サンプリングバッファの型

#[entry]
fn main() -> ! {
    let mut peripherals = Peripherals::take().unwrap();
    let core = CorePeripherals::take().unwrap();
    let mut clocks = GenericClockController::with_external_32kosc(
        peripherals.GCLK,
        &mut peripherals.MCLK,
        &mut peripherals.OSC32KCTRL,
        &mut peripherals.OSCCTRL,
        &mut peripherals.NVMCTRL,
    );

    // TODO: UARTドライバオブジェクトを初期化する
    let mut sets: Sets = Pins::new(peripherals.PORT).split();
    let mut delay = Delay::new(core.SYST, &mut clocks);

    let mut serial = sets.uart.init(
        &mut clocks,
        115200.hz(),
        peripherals.SERCOM2,
        &mut peripherals.MCLK,
        &mut sets.port,
    );

    let (microphone_adc, mut microphone_pin) = sets.microphone.init(
        peripherals.ADC1,
        &mut clocks,
        &mut peripherals.MCLK,
        &mut sets.port,
    );
    let mut microphone_adc: InterruptAdc<_, FreeRunning> = InterruptAdc::from(microphone_adc);
    // start ADC
    microphone_adc.start_conversion(&mut microphone_pin);

    // LCDの初期化
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

    // LCDのクリア（全体を黒で塗りつぶす）
    egrectangle!(
        top_left = (0, 0),
        bottom_right = (SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1),
        style = primitive_style!(fill_color = Rgb565::BLACK)
    )
    .draw(&mut display)
    .unwrap();
    // mater marker
    egrectangle!(
        top_left = (SCREEN_WIDTH/2-5,0),
        bottom_right = (SCREEN_WIDTH/2+5, 32),
        style = primitive_style!(fill_color = Rgb565::BLUE)
    )
    .draw(&mut display)
    .unwrap();

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
    writeln!(&mut serial, "start").unwrap();
    unsafe { NVIC::unmask(interrupt::ADC1_RESRDY); }
    writeln!(&mut serial, "check1").unwrap();

    let mut cntr = 0;
    let mut asum = 0;
    let mut anum = 0;

    loop {
        let processing_buffer = unsafe {
            let ctx = CTX.as_mut().unwrap();
            ctx.processing_buffer.as_mut().unwrap()
        };
        let len = processing_buffer.len();
        let cap = processing_buffer.capacity();
        if len == cap {
            let mut oarray: [u16; SMP_POINTS] = [0;SMP_POINTS];
            let mut barray: [u8; SMP_POINTS] = [0;SMP_POINTS];
            let (avg, flg) = normalization(&processing_buffer, &mut oarray);
            if flg == 1 {
                processing_buffer.clear();
                continue;
            }
            // make barray
            //get_barray(&processing_buffer, avg, &mut barray);
            get_barray(&oarray, avg, &mut barray);
            // Inflate 1
            inflate_barray(&mut barray, 1);
            inflate_barray(&mut barray, 1);
            // Inflate 0
            inflate_barray(&mut barray, 0);
            inflate_barray(&mut barray, 0);
            inflate_barray(&mut barray, 0);
            inflate_barray(&mut barray, 0);
            // Inflate 1
            inflate_barray(&mut barray, 1);
            inflate_barray(&mut barray, 1);
            // get rise edge period, count, max-min
            let (psum, pnum, pdiff) = get_rise_edge(&barray); 
            //let avg = psum as f32 / pnum as f32;

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
            //if true {
            if pdiff <= 100 {
                asum += psum;
                anum += pnum;
                cntr += 1;
                if cntr == 20 {
                    let avg2 = asum as f32 / anum as f32;
                    let freq = (ADC_SAMPLING_RATE - ADC_SAMPLING_ADJ) / avg2;
                    writeln!(&mut serial, "avg: {}",avg2).unwrap();
                    writeln!(&mut serial, "frequency: {}",freq).unwrap();
                    asum = 0;
                    anum = 0;
                    cntr = 0;

                    //let note = 14;
                    let (note,diff) = get_note_from_freq(440.0, freq);
                    draw_meter(&mut display, diff);
                    draw_note(&mut display, note);
                    draw_freq(&mut display, freq);
                    /*
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

fn normalization(iarray: &[u16], oarray: &mut [u16]) -> (f32, u32)  {
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
    let flg = if max - min > SMP_THRE { 1 } else { 0 };

    let mut sum = 0.0 as u32;
    for i in 0..SMP_POINTS-1 {
        let tmp = ((iarray[i] - min) as f32 * (0x00003fff as f32/(max - min) as f32)) as u16;
        oarray[i] = tmp;
        sum += tmp as u32;
    }
    (sum as f32 / SMP_POINTS as f32, flg)
}

fn get_barray(iarray: &[u16], avg: f32, barray: &mut [u8]) {
    for i in 3..SMP_POINTS-3 {
        let sum = iarray[i-3] as u32 + iarray[i-2] as u32 + iarray[i-1] as u32 + iarray[i] as u32
                              + iarray[i+1] as u32 + iarray[i+2] as u32 + iarray[i+3] as u32;
        let diff = sum as f32 / 7.0 - avg;
        if diff < 0.0 {
            barray[i] = 0;
        } else {
            barray[i] = 1;
        }
    }
}

fn inflate_barray(barray: &mut [u8], num: u8) {
    let mut carray: [u8; SMP_POINTS] = [0;SMP_POINTS];
    for i in 3..SMP_POINTS-3 {
        carray[i] = barray[i];
    }
    for i in 3..SMP_POINTS-4 {
        if (carray[i] == num)&(carray[i+1] != num) {
            barray[i+1] = num;
        }
        else if (carray[i] != num)&(carray[i+1] == num) {
            barray[i] = num;
        }
    }
}

fn get_rise_edge(barray: &[u8]) -> (u32, u32, u32) {
    // get rise edge placement
    // dont use final data
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
    // get rise edge period, count, max-min
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

const SCREEN_WIDTH: i32 = 320;
const SCREEN_HEIGHT: i32 = 240;
const FONT_WIDTH: i32 = 24;
const FONT_HEIGHT: i32 = 32;

fn draw_freq<T>(display: &mut T, freq: f32)
where
    T: embedded_graphics::DrawTarget<Rgb565>,
{
    // clear area
    egrectangle!(
        top_left = (0,192),
        bottom_right = (SCREEN_WIDTH-1, 224),
        style = primitive_style!(fill_color = Rgb565::BLACK)
    )
    .draw(display);

    // draw frequency
    let mut textbuffer = String::<U256>::new();
    write!(&mut textbuffer, "{:.2} Hz", freq).unwrap();
    //
    let length = textbuffer.len();
    //
    let left = SCREEN_WIDTH - (length as i32) * FONT_WIDTH;
    egtext!(
        text = textbuffer.as_str(),
        top_left = (left, 192),
        style = text_style!(font = Font24x32, text_color = Rgb565::WHITE)
    )
    .draw(display);
}

fn draw_note<T>(display: &mut T, note: u32)
where
    T: embedded_graphics::DrawTarget<Rgb565>,
{
    // clear area
    egrectangle!(
        top_left = (0,150),
        bottom_right = (SCREEN_WIDTH-1, 182),
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
        top_left = (left, 150),
        style = text_style!(font = Font24x32, text_color = Rgb565::WHITE)
    )
    .draw(display);
}

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

fn get_note_from_freq(fpitch: f32, freq: f32) -> (u32, f32) {
    let tone_num = 49.0 + 12.0 * log2f(freq / fpitch);
    let tone = (tone_num + 0.5) as u32;
    let diff = (tone_num - (tone as f32)) * (SCREEN_WIDTH as f32 - 20.0)
                + SCREEN_WIDTH as f32 / 2.0;
    return (tone,diff);
}

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
                    sampling_buffer.clear();
                }
            } else {
                let _ = sampling_buffer.push(sample);
            }
        }
    }
}


