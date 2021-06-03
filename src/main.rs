#![no_std]
#![no_main]

use panic_halt as _;
use wio_terminal as wio;

use core::fmt::Write;
use cortex_m::peripheral::NVIC;
use heapless::consts::*;
use heapless::Vec;
use heapless::String;
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
            // average
            let mut sum = 0 as u32;
            for i in 0..SMP_POINTS {
                sum += processing_buffer[i] as u32;
            }
            let avg = sum as f32/SMP_POINTS as f32;
            let mut barray: [u8; SMP_POINTS] = [0;SMP_POINTS];
            // make barray
            get_barray(&processing_buffer, avg, &mut barray);
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
            //writeln!(&mut serial, "Average: {}",avg).unwrap();
            //writeln!(&mut serial, "psum {}",psum).unwrap();
            //writeln!(&mut serial, "pnum {}",pnum).unwrap();
            //writeln!(&mut serial, "pdiff {}",pdiff).unwrap();
            if pdiff <= 100 {
                asum += psum;
                anum += pnum;
                cntr += 1;
                if cntr == 40 {
                    let avg = asum as f32 / anum as f32;
                    let freq = (ADC_SAMPLING_RATE - 365.0) / avg;
                    //let freq = (ADC_SAMPLING_RATE ) / avg;
                    writeln!(&mut serial, "avg: {}",avg).unwrap();
                    writeln!(&mut serial, "frequency: {}",freq).unwrap();
                    asum = 0;
                    anum = 0;
                    cntr = 0;

                    draw(&mut display, freq);

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

fn get_barray(iarray: &[u16], avg: f32, barray: &mut [u8]) {
    for i in 3..SMP_POINTS-3 {
        let sum = iarray[i-3] + iarray[i-2] + iarray[i-1] + iarray[i]
                              + iarray[i+1] + iarray[i+2] + iarray[i+3];
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

fn draw<T>(display: &mut T, freq: f32)
where
    T: embedded_graphics::DrawTarget<Rgb565>,
{
    // clear area
    const FONT_WIDTH: i32 = 24;
    const FONT_HEIGHT: i32 = 32;
    egrectangle!(
        top_left = (0,0),
        bottom_right = (SCREEN_WIDTH-1, FONT_HEIGHT),
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
        top_left = (left, 0),
        style = text_style!(font = Font24x32, text_color = Rgb565::WHITE)
    )
    .draw(display);
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
                }
            } else {
                let _ = sampling_buffer.push(sample);
            }
        }
    }
}


