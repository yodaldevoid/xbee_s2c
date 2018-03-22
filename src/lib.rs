//! Currently only supports XBee S2C hardware running the 802.15.04 RF firmware

#![no_std]

#[macro_use]
extern crate bitflags;
extern crate embedded_hal;
extern crate nb;

mod api_frame;

use api_frame::{FramePacker, TxOptions, TxRequestIter};

use embedded_hal::blocking::delay::DelayMs;
use embedded_hal::blocking::serial::Write as BlockingWrite;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::serial::{Read, Write};
use embedded_hal::spi::FullDuplex;

const BROADCAST_ADDR: u16 = 0xFFFF;
const COORDINATOR_ADDR: u16 = 0xFFFE;

// TODO: builders

// TODO: maybe add broadcast
// TODO: maybe add coordinator
pub enum Addr {
    Short(u16),
    Long(u64),
}

pub struct XBeeTransparent<'a, 'b, U: 'a, D: 'b> {
    serial: &'a mut U,
    timer: &'b mut D,
    cmd_char: u8,
    guard_time: u16,
}

trait XBeeApi {
    type Error;

    fn send_data(&mut self, frame_id: u8, addr: Addr, data: &[u8]) -> Result<(), Self::Error>;
    fn send_data_no_ack(&mut self, frame_id: u8, addr: Addr, data: &[u8]) -> Result<(), Self::Error>;
    fn at_command(&mut self, frame_id: u8, at_cmd: [u8; 2], params: &[u8]);
    fn at_queue_param(&mut self, frame_id: u8, at_cmd: [u8; 2], params: &[u8]);
    fn remote_at_command(&mut self, frame_id: u8, addr: Addr, at_cmd: [u8; 2], params: &[u8]);
}

pub struct XBeeApiUart<'a, U: 'a> {
    serial: &'a mut U,
}

pub struct XBeeApiEscapedUart<'a, U: 'a> {
    serial: &'a mut U,
}

pub struct XBeeApiSpi<'a, 'b, 'c, S: 'a, C: 'b, A: 'c> {
    serial: &'a mut S,
    cs: &'b mut C,
    attn: &'c mut A,
}

pub struct XBeeApiEscapedSpi<'a, 'b, 'c, S: 'a, C: 'b, A: 'c> {
    serial: &'a mut S,
    cs: &'b mut C,
    attn: &'c mut A,
}

impl<'a, 'b, E, U, D> XBeeTransparent<'a, 'b, U, D>
where
    U: Read<u8, Error = E> + BlockingWrite<u8, Error = E>,
    D: DelayMs<u16>,
{
    pub fn new(
        uart: &'a mut U,
        delay: &'b mut D,
        cmd_char: u8,
        guard_time: u16,
    ) -> XBeeTransparent<'a, 'b, U, D> {
        XBeeTransparent {
            serial: uart,
            timer: delay,
            cmd_char,
            guard_time,
        }
    }

    // TODO: maybe return result to show that the command has
    pub fn enter_command_mode(&mut self) -> Result<(), E> {
        // wait for guard time
        self.timer.delay_ms(self.guard_time);
        // send command character x3
        self.serial.bwrite_all(&[self.cmd_char; 3])?;
        // wait for "OK"
        loop {
            match self.serial.read() {
                Ok(b'O') => break,
                Ok(_) => panic!("Got other character while waiting for OK"), // TODO: error
                Err(nb::Error::WouldBlock) => {} // keep blocking
                Err(_) => panic!("Some error while waiting for OK"), // return Err(e.into()),
            }
        }
        loop {
            match self.serial.read() {
                Ok(b'K') => break,
                Ok(_) => panic!("Got other character while waiting for OK"), // TODO: error
                Err(nb::Error::WouldBlock) => {} // keep blocking
                Err(_) => panic!("Some error while waiting for OK"), // return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn to_api(self) -> XBeeApiUart<'a, U> {
        // TODO: AT command
        XBeeApiUart::new(self.serial)
    }
}

impl<'a, 'b, U, D> Read<u8> for XBeeTransparent<'a, 'b, U, D>
where
    U: Read<u8>,
{
    type Error = U::Error;

    fn read(&mut self) -> nb::Result<u8, Self::Error> {
        self.serial.read()
    }
}

impl<'a, 'b, U, D> Write<u8> for XBeeTransparent<'a, 'b, U, D>
where
    U: Write<u8>,
{
    type Error = U::Error;

    fn write(&mut self, word: u8) -> nb::Result<(), Self::Error> {
        self.serial.write(word)
    }

    fn flush(&mut self) -> nb::Result<(), Self::Error> {
        self.serial.flush()
    }
}

impl<'a, E, U> XBeeApiUart<'a, U>
where
    U: Read<u8, Error = E> + BlockingWrite<u8, Error = E>,
{
    pub fn new(uart: &'a mut U) -> XBeeApiUart<'a, U> {
        // TODO: check that we are in API mode and if not, switch
        XBeeApiUart{ serial: uart }
    }

    // TODO: set correct size for delay
    pub fn to_transpartent<'b, D>(
        self,
        delay: &'b mut D,
        cmd_char: u8,
        guard_time: u16,
    ) -> XBeeTransparent<'a, 'b, U, D>
    where
        D: DelayMs<u16>,
    {
        // TODO: AT command
        XBeeTransparent::new(self.serial, delay, cmd_char, guard_time)
    }
}

impl<'a, E, U> XBeeApi for XBeeApiUart<'a, U>
where
    U: Read<u8, Error = E> + BlockingWrite<u8, Error = E>,
{
    type Error = E;

    fn send_data(&mut self, frame_id: u8, addr: Addr, data: &[u8]) -> Result<(), E> {
        let tx_request = TxRequestIter::new(
            frame_id,
            addr,
            TxOptions::empty(),
            data.iter().map(|v| *v),
        );
        let frame = FramePacker::new(
            tx_request,
            false,
            false,
        ).expect("packing error"); // TODO:

        for byte in frame {
            self.serial.bwrite_all(&[byte])?;
        }
        self.serial.bflush()
    }

    fn send_data_no_ack(&mut self, frame_id: u8, addr: Addr, data: &[u8]) -> Result<(), E> {
        let tx_request = TxRequestIter::new(
            frame_id,
            addr,
            TxOptions::DISABLE_ACK,
            data.iter().map(|v| *v),
        );
        let frame = FramePacker::new(
            tx_request,
            false,
            false,
        ).expect("packing error"); // TODO:

        for byte in frame {
            self.serial.bwrite_all(&[byte])?;
        }
        self.serial.bflush()
    }

    fn at_command(&mut self, frame_id: u8, at_cmd: [u8; 2], params: &[u8]) {
        unimplemented!()
    }

    fn at_queue_param(&mut self, frame_id: u8, at_cmd: [u8; 2], params: &[u8]) {
        unimplemented!()
    }

    fn remote_at_command(&mut self, frame_id: u8, addr: Addr, at_cmd: [u8; 2], params: &[u8]) {
        unimplemented!()
    }
}

impl<'a, 'b, 'c, E, S, C, A> XBeeApiSpi<'a, 'b, 'c, S, C, A>
where
    S: FullDuplex<u8, Error = E>,
    C: OutputPin,
    A: InputPin,
{
    pub fn new_spi(
        spi: &'a mut S,
        cs: &'b mut C,
        attn: &'c mut A,
    ) -> XBeeApiSpi<'a, 'b, 'c, S, C, A> {
        XBeeApiSpi {
            serial: spi,
            cs,
            attn,
        }
    }
}
