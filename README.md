# gtuner_wio_terminal

Guiter tuner for wio-terminal with Rust

This Rust project is for (not only) guiter tuner on wio-terminal.

1.Instructions:

(1) Premise:: You already have Rust emvironment for wio-terminal. 
    Please see in Japanese:
        https://github.com/tomoyuki-nakabayashi/wio-examples-template
    Also there is a book for Rust on wio-terminal in Japanese.
        https://tomo-wait-for-it-yuki.hatenablog.com/entry/2021/04/04/140831
    This project use these environment and information.

(2) Clone this project.

(3) Run below command in gtune_wio_terminal.
    > cargo run --release
    Then compile project and write binary to wio-terminal

2.Usage:

(1) Power on wio-terminal and play tuning tone of insturment(like guiter).

(2) Basic pitch starts 440Hz and can be change 1Hz Up/Down by upper 2 buttons.

(3) Tune the instumet tone to become the upper meter keeping center of the meter.

3.Tuning program

There is a parameter to cariblating 440.0Hz center.
This program using ADC sampling rate (83333Hz) to get frequency. But *real* sampling
rate should be different because of individual difference. Then ADC_SAMPLING_ADJ
constant is added to adjut ADC sampling rate to *real* number.  Current number of
the ADC_SAMPLING_ADJ is 356 which is for my wio. So you need change the number if
you want to tuning accurately.

4.Otehrs
Rusti version 2018 0.1.0

@auther     Yoshiki Kurokawa <yoshiki.963@gmail.com>
@copylight  2021 Yoshiki Kurokawa
@license    https://opensource.org/licenses/MIT     MIT license
@version    0.1
