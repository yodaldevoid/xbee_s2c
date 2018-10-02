//! Currently only supports XBee S2C hardware running the 802.15.04 RF firmware

#![no_std]

extern crate arraydeque;
extern crate arrayvec;
#[macro_use]
extern crate bitflags;
extern crate embedded_hal;
#[macro_use]
extern crate nb;

pub mod api_frame;

use core::marker::PhantomData;

use api_frame::{
    ApiData,
    ApiUnpackError,
    FramePacker,
    TxOptions,
    TxRequestIter,
};

use arraydeque::ArrayDeque;
use arrayvec::{Array, ArrayVec};
use embedded_hal::blocking::delay::DelayMs;
use embedded_hal::blocking::serial::Write as BlockingWrite;
use embedded_hal::digital::{InputPin, OutputPin};
use embedded_hal::serial::{Read, Write};
use embedded_hal::spi::FullDuplex;

pub const BROADCAST_ADDR: u16 = 0xFFFF;
pub const COORDINATOR_ADDR: u16 = 0xFFFE;

trait XBeeQueue {
    fn remove_until_start(&mut self) -> Result<usize, ()>;
    fn remove_exact(&mut self, amount: usize) -> Result<(), ()>;
}

impl<A> XBeeQueue for ArrayVec<A>
where
    A: Array<Item=u8>,
{
    fn remove_until_start(&mut self) -> Result<usize, ()> {
        match self.iter().position(|c| c == &api_frame::START) {
            Some(size) => {
                self.remove_exact(size)?;
                Ok(size)
            },
            None => {
                let len = self.len();
                self.clear();
                Ok(len)
            }
        }
    }

    fn remove_exact(&mut self, amount: usize) -> Result<(), ()> {
        if amount == 0 {
            return Ok(())
        }

        if amount <= self.len() {
            self.drain(0..amount);
            Ok(())
        } else {
            Err(())
        }
    }
}

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

#[derive(Copy, Clone, Debug)]
pub enum XBeeApiError {
    Unpack(ApiUnpackError),
    Parse(()),
}

pub struct XBeeApiSpi<'a, 'b, 'c, SER: 'a, CS: 'b, ATTN: 'c> {
    serial: &'a mut SER,
    cs: Option<&'b mut CS>,
    attn: &'c mut ATTN,

    // TODO: make generic and allow passing in buffers
    tx_queue: ArrayDeque<[u8; 512]>,
    rx_queue: ArrayVec<[u8; 512]>,
}

impl<'a, 'b, SER_ERR, U, D> XBeeTransparent<'a, 'b, U, D>
where
    U: Read<u8, Error = SER_ERR> + BlockingWrite<u8, Error = SER_ERR>,
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
    pub fn enter_command_mode(&mut self) -> Result<(), SER_ERR> {
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

impl<'a, 'b, 'c, SER_ERR, SER, CS, ATTN> XBeeApiSpi<'a, 'b, 'c, SER, CS, ATTN>
where
    SER: FullDuplex<u8, Error = SER_ERR>,
    CS: OutputPin,
    ATTN: InputPin,
{
    pub fn new(
        spi: &'a mut SER,
        cs: Option<&'b mut CS>,
        attn: &'c mut ATTN,
    ) -> XBeeApiSpi<'a, 'b, 'c, SER, CS, ATTN> {
        XBeeApiSpi {
            serial: spi,
            cs,
            attn,
            tx_queue: ArrayDeque::new(),
            rx_queue: ArrayVec::new(),
        }
    }

    pub fn tx_queue_empty(&self) -> bool {
        self.tx_queue.is_empty()
    }

    pub fn tx_queue_full(&self) -> bool {
        self.tx_queue.is_full()
    }

    pub fn rx_queue_empty(&self) -> bool {
        self.rx_queue.is_empty()
    }

    pub fn rx_queue_full(&self) -> bool {
        self.rx_queue.is_full()
    }

    // TODO: differentiate between errors from reading and writing
    pub fn transmit_and_receive(&mut self) -> Result<bool, SER_ERR> {
        let mut val_read = false;
        let mut attn_val;
        while {
            attn_val = self.attn.is_high();
            !self.tx_queue.is_empty() || !attn_val
        } {
            let tx = if !self.tx_queue.is_empty() {
                // TODO: don't unwrap, pass up error
                self.tx_queue.pop_front().unwrap()
            } else {
                0xFF
            };

            // TODO: better error handling?
            block!(self.serial.send(tx))?;

            let rx = block!(self.serial.read())?;
            if !attn_val {
                // TODO: don't unwrap, pass up error
                self.rx_queue.try_push(rx).unwrap();
                val_read = true;
                if self.rx_queue.is_full() {
                    break;
                }
            }
        }

        Ok(val_read)
    }

    pub fn get_sender_receiver<'d>(&'d mut self) -> (XBeeApiSender<'d, SER_ERR>, XBeeApiReceiver<'d, SER_ERR>) {
        let tx_queue = &mut self.tx_queue;
        let rx_queue = &mut self.rx_queue;

        let sender = XBeeApiSender {
            tx_queue,
            _error: PhantomData,
        };
        let receiver = XBeeApiReceiver {
            rx_queue,
            _error: PhantomData,
        };

        (sender, receiver)
    }
}

#[derive(Debug)]
pub struct XBeeApiSender<'a, SER_ERR> {
    // TODO: make generic
    tx_queue: &'a mut ArrayDeque<[u8; 512]>,
    _error: PhantomData<*const SER_ERR>,
}

impl<'a, SER_ERR> XBeeApiSender<'a, SER_ERR> {
    pub fn queue_empty(&self) -> bool {
        self.tx_queue.is_empty()
    }

    pub fn queue_full(&self) -> bool {
        self.tx_queue.is_full()
    }

    pub fn send_data_raw(&mut self, data: &[u8]) -> Result<(), SER_ERR> {
        // TODO: error handling if we do not have enough space
        self.tx_queue.extend(data.iter().map(|&x| x));
        Ok(())
    }

    pub fn send_data(&mut self, frame_id: u8, addr: Addr, data: &[u8]) -> Result<(), SER_ERR> {
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

        // TODO: error handling if we do not have enough space
        self.tx_queue.extend(frame);
        Ok(())
    }

    pub fn send_data_no_ack(&mut self, frame_id: u8, addr: Addr, data: &[u8]) -> Result<(), SER_ERR> {
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

        // TODO: error handling if we do not have enough space
        self.tx_queue.extend(frame);
        Ok(())
    }

    pub fn at_command(&mut self, frame_id: u8, at_cmd: [u8; 2], params: &[u8]) {
        unimplemented!()
    }

    pub fn at_queue_param(&mut self, frame_id: u8, at_cmd: [u8; 2], params: &[u8]) {
        unimplemented!()
    }

    pub fn remote_at_command(&mut self, frame_id: u8, addr: Addr, at_cmd: [u8; 2], params: &[u8]) {
        unimplemented!()
    }
}

impl<'a, SER_ERR> Drop for XBeeApiSender<'a, SER_ERR> {
    fn drop(&mut self) {}
}

pub struct XBeeApiReceiver<'a, SER_ERR> {
    // TODO: make generic
    rx_queue: &'a mut ArrayVec<[u8; 512]>,
    _error: PhantomData<*const SER_ERR>,
}

impl<'a, SER_ERR> XBeeApiReceiver<'a, SER_ERR> {
    pub fn queue_empty(&self) -> bool {
        self.rx_queue.is_empty()
    }

    pub fn queue_full(&self) -> bool {
        self.rx_queue.is_full()
    }

    pub fn unpack_and_parse_buffer<'d>(&'d self) -> Result<ApiData<'d>, XBeeApiError> {
        let ret = match api_frame::unpack_frame(self.rx_queue.as_slice(), false, false) {
            Ok((frame, _rem)) => ApiData::parse(frame).map_err(|err| XBeeApiError::Parse(err)),
            Err(err) => Err(XBeeApiError::Unpack(err)),
        };

        ret
    }

    pub fn remove_until_packet(&mut self) -> Result<usize, ()>{
        self.rx_queue.remove_until_start()
    }

    pub fn remove_until_next_packet(&mut self) -> Result<usize, ()>{
        if let Some(_) = self.rx_queue.pop_at(0) {
            self.remove_until_packet().map(|len| len + 1)
        } else {
            Ok(0)
        }
    }
}

impl<'a, SER_ERR> Drop for XBeeApiReceiver<'a, SER_ERR> {
    fn drop(&mut self) {}
}
