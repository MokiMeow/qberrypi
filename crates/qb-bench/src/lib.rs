//! A DNS load generator: how many answers a second a server sustains, and how fast they are.
//!
//! It is the generator that qberrypi runs against itself over the board's loopback (the API's
//! `POST /api/qbp/bench`), and the `qb-bench` binary, built static for the Pi, runs the same loads
//! against Pi-hole, AdGuard Home, Unbound or Blocky on Raspberry Pi OS, so that the figures of every
//! server come from one generator measured one way. See `docs/bring-up/benchmark.md`.
//!
//! - [`wire`]: the bytes of a query and of a reply's header.
//! - [`names`]: the three classes of question, cached, blocked and forwarded.
//! - [`engine`]: the load at a set rate, with many questions in flight.
//! - [`run`]: the checks, the ramp and the steady load, and their reports.
//! - [`cpu`]: the processor time of the cores, the generator and the server.
//! - [`file`]: a file of runs, and two of them compared.

pub mod cpu;
pub mod engine;
pub mod figures;
pub mod file;
pub mod names;
pub mod run;
pub mod testserver;
pub mod wire;
