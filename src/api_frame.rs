use core::iter::ExactSizeIterator;

use super::Addr;

pub const START: u8 = 0x7E;
pub const ESCAPE: u8 = 0x7D;
pub const XON: u8 = 0x11;
pub const XOFF: u8 = 0x13;

#[derive(Debug)]
pub enum ApiPackError {
    TooShort,
    TooLong,
}

enum FramePackingState {
    Start,
    LenH,
    LenL,
    Data,
    //Checksum,
    Done,
}

// TODO: encryption
// TODO: escaped mode
pub struct FramePacker<I> {
    state: FramePackingState,
    escaped: bool,
    encrypted: bool,
    data: I,
    checksum: u8,
}

impl<I> FramePacker<I>
where
    I: ExactSizeIterator<Item = u8>,
{
    pub fn new(data: I, escaped: bool, encrypted: bool) -> Result<FramePacker<I>, ApiPackError> {
        if data.len() == 0 {
            return Err(ApiPackError::TooShort);
        }

        if data.len() > u16::max_value() as usize {
            return Err(ApiPackError::TooLong);
        }

        Ok(FramePacker {
            state: FramePackingState::Start,
            escaped,
            encrypted,
            data,
            checksum: 0,
        })
    }
}

impl<I> Iterator for FramePacker<I>
where
    I: ExactSizeIterator<Item = u8>,
{
    // TODO: get working with references
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            FramePackingState::Start => {
                self.state = FramePackingState::LenH;
                Some(START)
            }
            FramePackingState::LenH => {
                self.state = FramePackingState::LenL;
                Some((self.data.len() >> 8) as u8)
            }
            FramePackingState::LenL => {
                self.state = FramePackingState::Data;
                Some(self.data.len() as u8)
            }
            FramePackingState::Data => {
                if let Some(val) = self.data.next() {
                    self.checksum = self.checksum.wrapping_add(val);
                    Some(val)
                } else {
                    self.state = FramePackingState::Done;
                    Some(0xFF - self.checksum)
                }
            }
            FramePackingState::Done => None,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum ApiUnpackError {
    NoStart,
    BadLength(usize),
    BadChecksum(u8),
}

/// Returns the data portion of the frame and any remaining part of the buffer on success.
///
/// Currently escaped and encrypted modes are not supported.
pub fn unpack_frame(
    buf: &[u8],
    escaped: bool,
    _encryption: bool,
) -> Result<(&[u8], &[u8]), ApiUnpackError> {
    if buf.is_empty() {
        return Err(ApiUnpackError::NoStart);
    }

    if !escaped {
        if buf[0] != START {
            return Err(ApiUnpackError::NoStart);
        }
    } else {
        unimplemented!()
    }

    let buf = &buf[1..];

    let (len, buf) = buf.split_at(2);
    let len = ((len[0] as u16) << 8 | (len[1] as u16)) as usize;
    if len + 1 > buf.len() {
        return Err(ApiUnpackError::BadLength(3 + len + 1));
    }

    let (buf, rem) = buf.split_at(len + 1);

    let (checksum, data) = buf.split_last().unwrap();
    let check = data.iter().fold(0, |acc: u8, &val| acc.wrapping_add(val));
    if checksum.wrapping_add(check) == 0xFF {
        Ok((data, rem))
    } else {
        Err(ApiUnpackError::BadChecksum(check))
    }
}

bitflags! {
    pub struct TxOptions: u8 {
        const DISABLE_ACK = 0x01;
        const PAN_BROADCAST = 0x04;
    }
}

bitflags! {
    pub struct RxOptions: u8 {
        const ADDR_BROADCAST = 0x02;
        const PAN_BROADCAST = 0x04;
    }
}

/// bitfield
/// [0..2] reserved
/// [3..6] analog
/// [7..15] digital
// TODO: test if the reserved bits are on the top or bottom and what order
bitflags! {
    pub struct ChannelIndicator: u16 {
        const A3 = 0b0001000000000000;
        const A2 = 0b0000100000000000;
        const A1 = 0b0000010000000000;
        const A0 = 0b0000001000000000;
        const D8 = 0b0000000100000000;
        const D7 = 0b0000000010000000;
        const D6 = 0b0000000001000000;
        const D5 = 0b0000000000100000;
        const D4 = 0b0000000000010000;
        const D3 = 0b0000000000001000;
        const D2 = 0b0000000000000100;
        const D1 = 0b0000000000000010;
        const D0 = 0b0000000000000001;
    }
}

impl ChannelIndicator {
    fn contains_digital(&self) -> bool {
        self.contains(
            ChannelIndicator::D0
                | ChannelIndicator::D1
                | ChannelIndicator::D2
                | ChannelIndicator::D3
                | ChannelIndicator::D4
                | ChannelIndicator::D5
                | ChannelIndicator::D6
                | ChannelIndicator::D7
                | ChannelIndicator::D8,
        )
    }
}

#[derive(Debug, PartialEq)]
pub enum AtCommandStatus {
    Ok = 0,
    Error = 1,
    InvalidCommand = 2,
    InvalidParam = 3,
    NoResponse = 4,
    Unknown,
}

#[derive(Debug, PartialEq)]
pub enum TxStatus {
    Standard = 0x00,
    NoAck = 0x01,
    CcaFailure = 0x02,
    TxPurged = 0x03,
    NetworkAckFailure = 0x21,
    NotConnected = 0x22,
    InternalError = 0x31,
    ResourceDepletion = 0x32,
    PayloadTooLarge = 0x74,
    Unknown,
}

impl TxStatus {
    // Done instead of using the "From" trait to keep the conversion private
    fn from(val: u8) -> TxStatus {
        match val {
            0x00 => TxStatus::Standard,
            0x01 => TxStatus::NoAck,
            0x02 => TxStatus::CcaFailure,
            0x03 => TxStatus::TxPurged,
            0x21 => TxStatus::NetworkAckFailure,
            0x22 => TxStatus::NotConnected,
            0x31 => TxStatus::InternalError,
            0x32 => TxStatus::ResourceDepletion,
            0x74 => TxStatus::PayloadTooLarge,
            _ => TxStatus::Unknown,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum ModemStatus {
    HardwareReset = 0x00,
    WatchdogReset = 0x01,
    AssociatedCoordinator = 0x02,
    DissociatedCoordinator = 0x03,
    CoordinatorNewNetwork = 0x06,
    InputVoltageTooHigh = 0x0D,
    Unknown,
}

impl ModemStatus {
    fn from(val: u8) -> ModemStatus {
        match val {
            0x00 => ModemStatus::HardwareReset,
            0x01 => ModemStatus::WatchdogReset,
            0x02 => ModemStatus::AssociatedCoordinator,
            0x03 => ModemStatus::DissociatedCoordinator,
            0x06 => ModemStatus::CoordinatorNewNetwork,
            0x0D => ModemStatus::InputVoltageTooHigh,
            _ => ModemStatus::Unknown,
        }
    }
}

// TODO: maybe make sperate public faceing enums for send and recieve packets
#[derive(Debug, PartialEq)]
pub enum ApiData<'a> {
    // Send
    TxRequest64Addr {
        frame_id: u8,
        dest_addr: u64,
        options: TxOptions,
        data: &'a [u8],
    },
    // Send
    TxRequest16Addr {
        frame_id: u8,
        dest_addr: u16,
        options: TxOptions,
        data: &'a [u8],
    },
    // Send
    AtCommand {
        frame_id: u8,
        at_cmd: [u8; 2],
        params: &'a [u8],
    },
    // Send
    AtCommandQueueParam {
        frame_id: u8,
        at_cmd: [u8; 2],
        params: &'a [u8],
    },
    // Send
    RemoteAtCommand {
        frame_id: u8,
        // TODO: combine the addr into an enum
        dest_addr_64: u64,
        dest_addr_16: u16,
        at_cmd: [u8; 2],
        params: &'a [u8],
    },
    // Receive
    RxPacket64Addr {
        source_addr: u64,
        /// in -dBm
        rssi: u8,
        options: RxOptions,
        data: &'a [u8],
    },
    // Receive
    RxPacket16Addr {
        source_addr: u16,
        /// in -dBm
        rssi: u8,
        options: RxOptions,
        data: &'a [u8],
    },
    // Receive
    RxPacketIo64Addr {
        source_addr: u64,
        /// in -dBm
        rssi: u8,
        options: RxOptions,
        samples: u8,
        channel_indicator: ChannelIndicator,
        digital_samples: Option<u16>,
        analog_samples: [Option<u16>; 4],
    },
    // Receive
    RxPacketIo16Addr {
        source_addr: u16,
        /// in -dBm
        rssi: u8,
        options: RxOptions,
        samples: u8,
        channel_indicator: ChannelIndicator,
        digital_samples: Option<u16>,
        analog_samples: [Option<u16>; 4],
    },
    // Receive
    AtCommandResponse {
        frame_id: u8,
        at_cmd: [u8; 2],
        status: AtCommandStatus,
        data: &'a [u8],
    },
    // Receive
    TxStatus {
        frame_id: u8,
        status: TxStatus,
    },
    // Receive
    ModemStatus {
        status: ModemStatus,
    },
    // Receive
    RemoteAtCommandResponse {
        frame_id: u8,
        // TODO: combine the addr into an enum
        source_addr_64: u64,
        source_addr_16: u16,
        at_cmd: [u8; 2],
        status: AtCommandStatus,
        data: &'a [u8],
    },
}

impl<'a> ApiData<'a> {
    fn frame_type(&self) -> u8 {
        match *self {
            ApiData::TxRequest64Addr { .. } => 0x00,
            ApiData::TxRequest16Addr { .. } => 0x01,
            ApiData::AtCommand { .. } => 0x08,
            ApiData::AtCommandQueueParam { .. } => 0x09,
            ApiData::RemoteAtCommand { .. } => 0x17,
            ApiData::RxPacket64Addr { .. } => 0x80,
            ApiData::RxPacket16Addr { .. } => 0x81,
            ApiData::RxPacketIo64Addr { .. } => 0x82,
            ApiData::RxPacketIo16Addr { .. } => 0x83,
            ApiData::AtCommandResponse { .. } => 0x88,
            ApiData::TxStatus { .. } => 0x89,
            ApiData::ModemStatus { .. } => 0x8A,
            ApiData::RemoteAtCommandResponse { .. } => 0x97,
        }
    }

    pub fn parse<'b>(data: &'b [u8]) -> Result<ApiData<'b>, ()> {
        let len = data.len();
        let mut iter = data.iter();
        match *iter.next().unwrap() {
            // TODO: test if you can have en empty payload. Currently assumes no.
            0x00 if len > 10 => {
                let frame_id = *iter.next().unwrap();
                let dest_addr = ((*iter.next().unwrap() as u64) << 56)
                    | ((*iter.next().unwrap() as u64) << 48)
                    | ((*iter.next().unwrap() as u64) << 40)
                    | ((*iter.next().unwrap() as u64) << 32)
                    | ((*iter.next().unwrap() as u64) << 24)
                    | ((*iter.next().unwrap() as u64) << 16)
                    | ((*iter.next().unwrap() as u64) << 8)
                    | (*iter.next().unwrap() as u64);
                let options = TxOptions::from_bits_truncate(*iter.next().unwrap());

                Ok(ApiData::TxRequest64Addr {
                    frame_id,
                    dest_addr,
                    options,
                    data: iter.as_slice(),
                })
            }
            // TODO: test if you can have en empty payload
            0x01 if len > 4 => {
                let frame_id = *iter.next().unwrap();
                let dest_addr =
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16);
                let options = TxOptions::from_bits_truncate(*iter.next().unwrap());

                Ok(ApiData::TxRequest16Addr {
                    frame_id,
                    dest_addr,
                    options,
                    data: iter.as_slice(),
                })
            }
            0x08 if len > 3 => {
                let frame_id = *iter.next().unwrap();
                let at_cmd = [*iter.next().unwrap(), *iter.next().unwrap()];

                Ok(ApiData::AtCommand {
                    frame_id,
                    at_cmd,
                    params: iter.as_slice(),
                })
            }
            0x09 if len > 3 => {
                let frame_id = *iter.next().unwrap();
                let at_cmd = [*iter.next().unwrap(), *iter.next().unwrap()];

                Ok(ApiData::AtCommandQueueParam {
                    frame_id,
                    at_cmd,
                    params: iter.as_slice(),
                })
            }
            0x17 if len > 13 => {
                let frame_id = *iter.next().unwrap();
                let dest_addr_64 = ((*iter.next().unwrap() as u64) << 56)
                    | ((*iter.next().unwrap() as u64) << 48)
                    | ((*iter.next().unwrap() as u64) << 40)
                    | ((*iter.next().unwrap() as u64) << 32)
                    | ((*iter.next().unwrap() as u64) << 24)
                    | ((*iter.next().unwrap() as u64) << 16)
                    | ((*iter.next().unwrap() as u64) << 8)
                    | (*iter.next().unwrap() as u64);
                let dest_addr_16 =
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16);
                let at_cmd = [*iter.next().unwrap(), *iter.next().unwrap()];

                Ok(ApiData::RemoteAtCommand {
                    frame_id,
                    dest_addr_64,
                    dest_addr_16,
                    at_cmd,
                    params: iter.as_slice(),
                })
            }
            0x80 if len > 10 => {
                let source_addr = ((*iter.next().unwrap() as u64) << 56)
                    | ((*iter.next().unwrap() as u64) << 48)
                    | ((*iter.next().unwrap() as u64) << 40)
                    | ((*iter.next().unwrap() as u64) << 32)
                    | ((*iter.next().unwrap() as u64) << 24)
                    | ((*iter.next().unwrap() as u64) << 16)
                    | ((*iter.next().unwrap() as u64) << 8)
                    | (*iter.next().unwrap() as u64);
                let rssi = *iter.next().unwrap();
                let options = RxOptions::from_bits_truncate(*iter.next().unwrap());

                Ok(ApiData::RxPacket64Addr {
                    source_addr,
                    rssi,
                    options,
                    data: iter.as_slice(),
                })
            }
            0x81 if len > 4 => {
                let source_addr =
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16);
                let rssi = *iter.next().unwrap();
                let options = RxOptions::from_bits_truncate(*iter.next().unwrap());

                Ok(ApiData::RxPacket16Addr {
                    source_addr,
                    rssi,
                    options,
                    data: iter.as_slice(),
                })
            }
            0x82 if len > 13 => {
                let source_addr = ((*iter.next().unwrap() as u64) << 56)
                    | ((*iter.next().unwrap() as u64) << 48)
                    | ((*iter.next().unwrap() as u64) << 40)
                    | ((*iter.next().unwrap() as u64) << 32)
                    | ((*iter.next().unwrap() as u64) << 24)
                    | ((*iter.next().unwrap() as u64) << 16)
                    | ((*iter.next().unwrap() as u64) << 8)
                    | (*iter.next().unwrap() as u64);
                let rssi = *iter.next().unwrap();
                let options = RxOptions::from_bits_truncate(*iter.next().unwrap());
                let samples = *iter.next().unwrap();
                let channel_indicator = ChannelIndicator::from_bits_truncate(
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16),
                );
                let digital_samples = if channel_indicator.contains_digital() {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog0 = if channel_indicator.contains(ChannelIndicator::A0) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog1 = if channel_indicator.contains(ChannelIndicator::A1) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog2 = if channel_indicator.contains(ChannelIndicator::A2) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog3 = if channel_indicator.contains(ChannelIndicator::A3) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog_samples = [analog0, analog1, analog2, analog3];

                Ok(ApiData::RxPacketIo64Addr {
                    source_addr,
                    rssi,
                    options,
                    samples,
                    channel_indicator,
                    digital_samples,
                    analog_samples,
                })
            }
            0x83 if len > 7 => {
                let source_addr =
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16);
                let rssi = *iter.next().unwrap();
                let options = RxOptions::from_bits_truncate(*iter.next().unwrap());
                let samples = *iter.next().unwrap();
                let channel_indicator = ChannelIndicator::from_bits_truncate(
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16),
                );
                let digital_samples = if channel_indicator.contains_digital() {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog0 = if channel_indicator.contains(ChannelIndicator::A0) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog1 = if channel_indicator.contains(ChannelIndicator::A1) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog2 = if channel_indicator.contains(ChannelIndicator::A2) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog3 = if channel_indicator.contains(ChannelIndicator::A3) {
                    Some(((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16))
                } else {
                    None
                };
                let analog_samples = [analog0, analog1, analog2, analog3];

                Ok(ApiData::RxPacketIo16Addr {
                    source_addr,
                    rssi,
                    options,
                    samples,
                    channel_indicator,
                    digital_samples,
                    analog_samples,
                })
            }
            0x88 if len > 4 => {
                let frame_id = *iter.next().unwrap();
                let at_cmd = [*iter.next().unwrap(), *iter.next().unwrap()];
                let status = match *iter.next().unwrap() {
                    0 => AtCommandStatus::Ok,
                    1 => AtCommandStatus::Error,
                    2 => AtCommandStatus::InvalidCommand,
                    3 => AtCommandStatus::InvalidParam,
                    _ => AtCommandStatus::Unknown,
                };

                Ok(ApiData::AtCommandResponse {
                    frame_id,
                    at_cmd,
                    status,
                    data: iter.as_slice(),
                })
            }
            0x89 if len == 3 => {
                let frame_id = *iter.next().unwrap();
                let status = TxStatus::from(*iter.next().unwrap());

                Ok(ApiData::TxStatus { frame_id, status })
            }
            0x8A if len == 2 => {
                let status = ModemStatus::from(*iter.next().unwrap());

                Ok(ApiData::ModemStatus { status })
            }
            0x97 if len > 14 => {
                let frame_id = *iter.next().unwrap();
                let source_addr_64 = ((*iter.next().unwrap() as u64) << 56)
                    | ((*iter.next().unwrap() as u64) << 48)
                    | ((*iter.next().unwrap() as u64) << 40)
                    | ((*iter.next().unwrap() as u64) << 32)
                    | ((*iter.next().unwrap() as u64) << 24)
                    | ((*iter.next().unwrap() as u64) << 16)
                    | ((*iter.next().unwrap() as u64) << 8)
                    | (*iter.next().unwrap() as u64);
                let source_addr_16 =
                    ((*iter.next().unwrap() as u16) << 8) | (*iter.next().unwrap() as u16);
                let at_cmd = [*iter.next().unwrap(), *iter.next().unwrap()];
                let status = match *iter.next().unwrap() {
                    0 => AtCommandStatus::Ok,
                    1 => AtCommandStatus::Error,
                    2 => AtCommandStatus::InvalidCommand,
                    3 => AtCommandStatus::InvalidParam,
                    4 => AtCommandStatus::NoResponse,
                    _ => AtCommandStatus::Unknown,
                };

                Ok(ApiData::RemoteAtCommandResponse {
                    frame_id,
                    source_addr_64,
                    source_addr_16,
                    at_cmd,
                    status,
                    data: iter.as_slice(),
                })
            }
            _ => Err(()),
        }
    }
}

enum TxRequestState {
    FrameType,
    FrameId,
    Addr,
    Options,
    Data,
}

pub struct TxRequestIter<I> {
    state: TxRequestState,
    frame_id: u8,
    addr: u64,
    addr_shift: u8,
    options: TxOptions,
    data: I,
}

impl<I> TxRequestIter<I>
where
    I: ExactSizeIterator<Item = u8>,
{
    pub fn new(frame_id: u8, addr: Addr, options: TxOptions, data: I) -> TxRequestIter<I> {
        let (addr, addr_shift) = match addr {
            Addr::Long(addr) => (addr, 56),
            Addr::Short(addr) => (addr as u64, 8),
        };

        TxRequestIter {
            state: TxRequestState::FrameType,
            frame_id,
            addr,
            addr_shift,
            options,
            data,
        }
    }
}

impl<I> Iterator for TxRequestIter<I>
where
    I: ExactSizeIterator<Item = u8>,
{
    // TODO: get working with references
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            TxRequestState::FrameType => {
                self.state = TxRequestState::FrameId;
                match self.addr_shift {
                    56 => Some(0x00), // TxRequest64Addr
                    8 => Some(0x01),  // TxRequest16Addr
                    _ => unreachable!(),
                }
            }
            TxRequestState::FrameId => {
                self.state = TxRequestState::Addr;
                Some(self.frame_id)
            }
            TxRequestState::Addr => {
                let val = (self.addr >> self.addr_shift) as u8;
                if self.addr_shift == 0 {
                    self.state = TxRequestState::Options;
                } else {
                    self.addr_shift -= 8;
                }
                Some(val)
            }
            TxRequestState::Options => {
                self.state = TxRequestState::Data;
                Some(self.options.bits())
            }
            TxRequestState::Data => self.data.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let size = match self.state {
            TxRequestState::FrameType => {
                2 + ((self.addr_shift as usize) / 8 + 1) + 1 + self.data.len()
            }
            TxRequestState::FrameId => {
                1 + ((self.addr_shift as usize) / 8 + 1) + 1 + self.data.len()
            }
            TxRequestState::Addr => ((self.addr_shift as usize) / 8 + 1) + 1 + self.data.len(),
            TxRequestState::Options => 1 + self.data.len(),
            TxRequestState::Data => self.data.len(),
        };

        (size, Some(size))
    }
}

impl<I> ExactSizeIterator for TxRequestIter<I>
where
    I: ExactSizeIterator<Item = u8>,
{
    fn len(&self) -> usize {
        match self.state {
            TxRequestState::FrameType => {
                2 + ((self.addr_shift as usize) / 8 + 1) + 1 + self.data.len()
            }
            TxRequestState::FrameId => {
                1 + ((self.addr_shift as usize) / 8 + 1) + 1 + self.data.len()
            }
            TxRequestState::Addr => ((self.addr_shift as usize) / 8 + 1) + 1 + self.data.len(),
            TxRequestState::Options => 1 + self.data.len(),
            TxRequestState::Data => self.data.len(),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn at_command_nh_parse_test() {
        let unpacked_data = [0x08, 0x52, 0x4E, 0x48];
        let parsed_data = ApiData::parse(&unpacked_data[..]).unwrap();

        let test_data = ApiData::AtCommand {
            frame_id: 0x52,
            at_cmd: [b'N', b'H'],
            params: &[],
        };

        assert_eq!(parsed_data, test_data);
    }

    #[test]
    fn at_command_dl_parse_test() {
        let unpacked_data = [0x08, 0x4D, 0x44, 0x4C, 0x00, 0x00, 0x0F, 0xFF];
        let parsed_data = ApiData::parse(&unpacked_data[..]).unwrap();

        let test_data = ApiData::AtCommand {
            frame_id: 0x4D,
            at_cmd: [b'D', b'L'],
            params: &[0x00, 0x00, 0x0F, 0xFF],
        };

        assert_eq!(parsed_data, test_data);
    }

    #[test]
    fn at_commmand_response_bd_parse_test() {
        let unpacked_data = [0x88, 0x01, 0x42, 0x44, 0x00];
        let parsed_data = ApiData::parse(&unpacked_data[..]).unwrap();

        let test_data = ApiData::AtCommandResponse {
            frame_id: 0x01,
            at_cmd: [b'B', b'D'],
            status: AtCommandStatus::Ok,
            data: &[],
        };

        assert_eq!(parsed_data, test_data);
    }

    #[test]
    fn tx_status_parse_test() {
        let unpacked_data = [0x89, 0x01, 0x00];
        let parsed_data = ApiData::parse(&unpacked_data[..]).unwrap();

        let test_data = ApiData::TxStatus {
            frame_id: 0x01,
            status: TxStatus::Standard,
        };

        assert_eq!(parsed_data, test_data);
    }

    #[test]
    fn modem_status_parse_test() {
        let unpacked_data = [0x8A, 0x00];
        let parsed_data = ApiData::parse(&unpacked_data[..]).unwrap();

        let test_data = ApiData::ModemStatus {
            status: ModemStatus::HardwareReset,
        };

        assert_eq!(parsed_data, test_data);
    }

    #[test]
    fn unpack_frame_test() {
        let frame = [
            0x7E, 0x00, 0x0A, 0x01, 0x01, 0x50, 0x01, 0x00, 0x48, 0x65, 0x6C, 0x6C, 0x6F, 0xB8,
            // extra data
            0x7E, 0x01, 0x02,
        ];
        let (unpacked_data, rem) = unpack_frame(&frame[..], false, false).unwrap();
        assert_eq!(
            unpacked_data,
            &[0x01, 0x01, 0x50, 0x01, 0x00, 0x48, 0x65, 0x6C, 0x6C, 0x6F]
        );
        assert_eq!(rem, &[0x7E, 0x01, 0x02]);
    }

    #[test]
    fn create_tx_request_test() {
        use arrayvec::ArrayVec;

        #[cfg_attr(rustfmt, rustfmt_skip)]
        let frame = [
            0x00,
            0x01,
            0x00, 0x13, 0xA2, 0x00, 0x41, 0x5D, 0x1D, 0xBB,
            0x00,
            0x54, 0x65, 0x73, 0x74, 0x69, 0x6E, 0x67,
        ];
        let mut vec: ArrayVec<[u8; 32]> = ArrayVec::new();
        let tx_request = TxRequestIter::new(
            1,
            Addr::Long(0x0013_A200_415D_1DBB),
            TxOptions::empty(),
            b"Testing".iter().map(|v| *v),
        );
        vec.extend(tx_request);
        assert_eq!(vec.as_slice(), &frame[..]);
    }

    #[test]
    fn packing_test() {
        use arrayvec::ArrayVec;

        #[cfg_attr(rustfmt, rustfmt_skip)]
        let data = [
            0x00,
            0x01,
            0x00, 0x13, 0xA2, 0x00, 0x41, 0x5D, 0x1D, 0xBB,
            0x00,
            0x54, 0x65, 0x73, 0x74, 0x69, 0x6E, 0x67,
        ];
        #[cfg_attr(rustfmt, rustfmt_skip)]
        let test_frame = [
            0x7E,
            0x00, 0x12,
            0x00,
            0x01,
            0x00, 0x13, 0xA2, 0x00, 0x41, 0x5D, 0x1D, 0xBB,
            0x00,
            0x54, 0x65, 0x73, 0x74, 0x69, 0x6E, 0x67,
            0xF5,
        ];
        let mut vec: ArrayVec<[u8; 32]> = ArrayVec::new();
        let packed_frame =
            FramePacker::new(data.iter().map(|v| *v), false, false).expect("packing error");
        vec.extend(packed_frame);
        assert_eq!(vec.as_slice(), &test_frame[..]);
    }
}
