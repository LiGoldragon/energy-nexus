//! Contract admission owns the connection before any payload can be restored.
use crate::Framing;
use signal::{
    ByteViewable, ContractDigest, Contracted, Handshake, HandshakeReceipt, HandshakeRejection,
    Restorable, Signal, Signalizable,
};
use std::{io, marker::PhantomData, os::unix::net::UnixStream};

/// A connection whose first frame has not yet established its compiled root.
pub struct BinaryConnection<C> {
    stream: UnixStream,
    contract: PhantomData<C>,
}

/// The stream and its kernel peer after the root's greeting was accepted.
pub struct AdmittedConnection<C> {
    stream: UnixStream,
    peer: nix::sys::socket::UnixCredentials,
    contract: PhantomData<C>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionStage {
    PeerCredentials,
    GreetingRead,
    GreetingWrite,
    ReceiptRead,
    ReceiptWrite,
}

#[derive(Debug)]
pub enum AdmissionRefusal {
    Io {
        stage: AdmissionStage,
        source: io::Error,
    },
    MalformedHandshake,
    MalformedReceipt,
    Encoding,
    Mismatch {
        expected: ContractDigest,
        observed: ContractDigest,
        receipt_failure: Option<io::Error>,
    },
    Refused {
        expected: ContractDigest,
        rejection: HandshakeRejection,
    },
    UnexpectedReceipt {
        expected: ContractDigest,
        observed: ContractDigest,
    },
}

#[derive(Debug)]
pub enum PayloadRefusal {
    Io(io::Error),
    Malformed,
    Encoding,
}

impl<C> From<UnixStream> for BinaryConnection<C> {
    fn from(stream: UnixStream) -> Self {
        Self {
            stream,
            contract: PhantomData,
        }
    }
}

pub trait Admittable<C>: Sized {
    fn admit(self) -> Result<AdmittedConnection<C>, AdmissionRefusal>;
    fn greet(self) -> Result<AdmittedConnection<C>, AdmissionRefusal>;
}

impl<C: Contracted> Admittable<C> for BinaryConnection<C> {
    fn admit(mut self) -> Result<AdmittedConnection<C>, AdmissionRefusal> {
        let peer =
            nix::sys::socket::getsockopt(&self.stream, nix::sys::socket::sockopt::PeerCredentials)
                .map_err(|source| AdmissionRefusal::Io {
                    stage: AdmissionStage::PeerCredentials,
                    source: source.into(),
                })?;
        let bytes = self
            .stream
            .read_frame()
            .map_err(|source| AdmissionRefusal::Io {
                stage: AdmissionStage::GreetingRead,
                source,
            })?;
        let greeting = Signal::<Handshake>::from(bytes)
            .restore()
            .map_err(|_| AdmissionRefusal::MalformedHandshake)?;
        let receipt = C::receipt(&greeting)
            .signalize()
            .map_err(|_| AdmissionRefusal::Encoding)?;
        let written = self.stream.write_frame(receipt.bytes());
        let expected = C::contract_digest();
        if greeting.contract_digest != expected {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
            return Err(AdmissionRefusal::Mismatch {
                expected,
                observed: greeting.contract_digest,
                receipt_failure: written.err(),
            });
        }
        written.map_err(|source| AdmissionRefusal::Io {
            stage: AdmissionStage::ReceiptWrite,
            source,
        })?;
        Ok(AdmittedConnection {
            stream: self.stream,
            peer,
            contract: PhantomData,
        })
    }

    fn greet(mut self) -> Result<AdmittedConnection<C>, AdmissionRefusal> {
        let peer =
            nix::sys::socket::getsockopt(&self.stream, nix::sys::socket::sockopt::PeerCredentials)
                .map_err(|source| AdmissionRefusal::Io {
                    stage: AdmissionStage::PeerCredentials,
                    source: source.into(),
                })?;
        let greeting = C::greeting()
            .signalize()
            .map_err(|_| AdmissionRefusal::Encoding)?;
        self.stream
            .write_frame(greeting.bytes())
            .map_err(|source| AdmissionRefusal::Io {
                stage: AdmissionStage::GreetingWrite,
                source,
            })?;
        let bytes = self
            .stream
            .read_frame()
            .map_err(|source| AdmissionRefusal::Io {
                stage: AdmissionStage::ReceiptRead,
                source,
            })?;
        let receipt = Signal::<HandshakeReceipt>::from(bytes)
            .restore()
            .map_err(|_| AdmissionRefusal::MalformedReceipt)?;
        let expected = C::contract_digest();
        match receipt {
            HandshakeReceipt::Greeted(observed) if observed == expected => Ok(AdmittedConnection {
                stream: self.stream,
                peer,
                contract: PhantomData,
            }),
            HandshakeReceipt::Greeted(observed) => {
                Err(AdmissionRefusal::UnexpectedReceipt { expected, observed })
            }
            HandshakeReceipt::GreetingRefused(rejection) => Err(AdmissionRefusal::Refused {
                expected,
                rejection,
            }),
        }
    }
}

pub trait PeerIdentifiable {
    fn peer(&self) -> nix::sys::socket::UnixCredentials;
}
impl<C> PeerIdentifiable for AdmittedConnection<C> {
    fn peer(&self) -> nix::sys::socket::UnixCredentials {
        self.peer
    }
}

pub trait PayloadReceivable<C> {
    fn receive(&mut self) -> Result<C, PayloadRefusal>;
}
impl<C> PayloadReceivable<C> for AdmittedConnection<C>
where
    Signal<C>: Restorable<C>,
{
    fn receive(&mut self) -> Result<C, PayloadRefusal> {
        let bytes = self.stream.read_frame().map_err(PayloadRefusal::Io)?;
        Signal::<C>::from(bytes)
            .restore()
            .map_err(|_| PayloadRefusal::Malformed)
    }
}

pub trait PayloadSendable {
    fn send<M: Signalizable>(&mut self, message: &M) -> Result<(), PayloadRefusal>;
}
impl<C> PayloadSendable for AdmittedConnection<C> {
    fn send<M: Signalizable>(&mut self, message: &M) -> Result<(), PayloadRefusal> {
        let signal = message.signalize().map_err(|_| PayloadRefusal::Encoding)?;
        self.stream
            .write_frame(signal.bytes())
            .map_err(PayloadRefusal::Io)
    }
}
