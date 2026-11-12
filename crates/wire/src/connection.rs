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
struct AdmittedConnection<C> {
    stream: UnixStream,
    peer: nix::sys::socket::UnixCredentials,
    contract: PhantomData<C>,
}

/// Incoming side: receive only this contract's request; send only its response.
pub struct AdmittedServerConnection<C> {
    connection: AdmittedConnection<C>,
}
/// Outgoing side: send only this contract's request; receive only its response.
pub struct AdmittedClientConnection<C> {
    connection: AdmittedConnection<C>,
}

/// One compiled request root determines its reply vocabulary on both sides.
pub trait ExchangeContract: Contracted {
    type Response;
}
impl ExchangeContract for signal_energy_nexus::Call {
    type Response = signal_energy_nexus::Response;
}
impl ExchangeContract for meta_signal_energy_nexus::Call {
    type Response = meta_signal_energy_nexus::Response;
}
impl ExchangeContract for meta_signal_energy_nexus::internal::Call {
    type Response = meta_signal_energy_nexus::internal::Response;
}
impl ExchangeContract for energy_nexus_emission::Query {
    type Response = energy_nexus_emission::Response;
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
    fn admit(self) -> Result<AdmittedServerConnection<C>, AdmissionRefusal>;
    fn greet(self) -> Result<AdmittedClientConnection<C>, AdmissionRefusal>;
}

impl<C: ExchangeContract> Admittable<C> for BinaryConnection<C> {
    fn admit(mut self) -> Result<AdmittedServerConnection<C>, AdmissionRefusal> {
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
        Ok(AdmittedServerConnection {
            connection: AdmittedConnection {
                stream: self.stream,
                peer,
                contract: PhantomData,
            },
        })
    }

    fn greet(mut self) -> Result<AdmittedClientConnection<C>, AdmissionRefusal> {
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
            HandshakeReceipt::Greeted(observed) if observed == expected => {
                Ok(AdmittedClientConnection {
                    connection: AdmittedConnection {
                        stream: self.stream,
                        peer,
                        contract: PhantomData,
                    },
                })
            }
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
impl<C> PeerIdentifiable for AdmittedServerConnection<C> {
    fn peer(&self) -> nix::sys::socket::UnixCredentials {
        self.connection.peer
    }
}
impl<C> PeerIdentifiable for AdmittedClientConnection<C> {
    fn peer(&self) -> nix::sys::socket::UnixCredentials {
        self.connection.peer
    }
}

// The private frame codec is shared; only the role-specific owners expose it.
trait MessageReceivable<M> {
    fn receive_message(&mut self) -> Result<M, PayloadRefusal>;
}
impl<C, M> MessageReceivable<M> for AdmittedConnection<C>
where
    Signal<M>: Restorable<M>,
{
    fn receive_message(&mut self) -> Result<M, PayloadRefusal> {
        let bytes = self.stream.read_frame().map_err(PayloadRefusal::Io)?;
        Signal::<M>::from(bytes)
            .restore()
            .map_err(|_| PayloadRefusal::Malformed)
    }
}
trait MessageSendable<M> {
    fn send_message(&mut self, message: &M) -> Result<(), PayloadRefusal>;
}
impl<C, M: Signalizable> MessageSendable<M> for AdmittedConnection<C> {
    fn send_message(&mut self, message: &M) -> Result<(), PayloadRefusal> {
        let signal = message.signalize().map_err(|_| PayloadRefusal::Encoding)?;
        self.stream
            .write_frame(signal.bytes())
            .map_err(PayloadRefusal::Io)
    }
}

pub trait RequestReceivable<C> {
    fn receive_request(&mut self) -> Result<C, PayloadRefusal>;
}
impl<C: ExchangeContract> RequestReceivable<C> for AdmittedServerConnection<C>
where
    Signal<C>: Restorable<C>,
{
    fn receive_request(&mut self) -> Result<C, PayloadRefusal> {
        self.connection.receive_message()
    }
}
pub trait ResponseSendable<C: ExchangeContract> {
    fn send_response(&mut self, response: &C::Response) -> Result<(), PayloadRefusal>;
}
impl<C: ExchangeContract> ResponseSendable<C> for AdmittedServerConnection<C>
where
    C::Response: Signalizable,
{
    fn send_response(&mut self, response: &C::Response) -> Result<(), PayloadRefusal> {
        self.connection.send_message(response)
    }
}
pub trait RequestSendable<C> {
    fn send_request(&mut self, request: &C) -> Result<(), PayloadRefusal>;
}
impl<C: ExchangeContract + Signalizable> RequestSendable<C> for AdmittedClientConnection<C> {
    fn send_request(&mut self, request: &C) -> Result<(), PayloadRefusal> {
        self.connection.send_message(request)
    }
}
pub trait ResponseReceivable<C: ExchangeContract> {
    fn receive_response(&mut self) -> Result<C::Response, PayloadRefusal>;
}
impl<C: ExchangeContract> ResponseReceivable<C> for AdmittedClientConnection<C>
where
    Signal<C::Response>: Restorable<C::Response>,
{
    fn receive_response(&mut self) -> Result<C::Response, PayloadRefusal> {
        self.connection.receive_message()
    }
}
