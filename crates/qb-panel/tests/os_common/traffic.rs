//! The mockup's fake traffic, which the budget tests replay: bursty Poisson arrivals, from `common.js`
//! `Sim`, with its public ad names, its example sites and the house's six devices that ask.
//!
//! The Drain's budget and the OS screens' budget run the same model, so the same seed is the same afternoon.

use qb_panel::{Share, TimeOfDay};

/// The share of queries that are ads, as the mockup's traffic has them.
const AD_SHARE: f64 = 0.44;

/// The last octets of the house's devices that ask, as the mockup's `CLIENTS`: Desk PC, Sam's phone, the
/// living room TV, the kid's tablet, the printer and the doorbell.
pub const CLIENTS: [u8; 6] = [12, 23, 31, 40, 57, 64];

/// The public ad names of the mockup's traffic.
pub const ADS: [&str; 16] = [
    "doubleclick.net",
    "googlesyndication.com",
    "adnxs.com",
    "criteo.com",
    "taboola.com",
    "outbrain.com",
    "moatads.com",
    "adsrvr.org",
    "pubmatic.com",
    "scorecardresearch.com",
    "amazon-adsystem.com",
    "rubiconproject.com",
    "app-measurement.com",
    "ads.yahoo.com",
    "casalemedia.com",
    "openx.net",
];

/// Ordinary sites, all under example.org.
pub const SITES: [&str; 16] = [
    "www", "mail", "api", "cdn", "static", "login", "video", "images", "docs", "files", "news",
    "music", "maps", "shop", "chat", "time",
];

/// The mulberry32 generator, as the mockup's `rng`.
struct Rng(u32);

impl Rng {
    /// The next number in 0 to 1, 1 not included.
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6D2B_79F5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        f64::from(t ^ (t >> 14)) / 4_294_967_296.0
    }
}

/// A query of the traffic.
pub struct Query {
    /// The name asked for.
    pub name: String,
    /// Whether it is an ad, one the lists have.
    pub ad: bool,
    /// Who asked, as the last octet of the device's address.
    pub client: u8,
}

/// The mockup's fake traffic: bursty Poisson arrivals, from `common.js` `Sim`.
pub struct Sim {
    /// The random numbers.
    rng: Rng,
    /// The time of day, in seconds.
    pub clock: f64,
    /// Whether blocking is paused: ads are answered then.
    pub paused: bool,
    /// Queries so far today.
    pub queries: u32,
    /// Queries blocked among them.
    pub blocked: u32,
    /// Seconds until the next query.
    pub next: f64,
    /// Queries left in the burst under way.
    burst: u32,
}

impl Sim {
    /// The traffic of a seed, starting at 10:21 with the day's figures of the mockup's stills.
    pub fn new(seed: u32, paused: bool) -> Self {
        Self {
            rng: Rng(seed),
            clock: f64::from(10 * 3600 + 21 * 60),
            paused,
            queries: 10_818,
            blocked: 4_780,
            next: 0.4,
            burst: 0,
        }
    }

    /// One query.
    fn generate(&mut self) -> Query {
        let ad = self.rng.next() < AD_SHARE;
        let pick = (self.rng.next() * 16.0) as usize;
        let name = if ad {
            ADS[pick % 16].to_string()
        } else {
            format!("{}.example.org", SITES[pick % 16])
        };
        // The mockup also picks a client and an answer time; the generator keeps the answer time in step.
        let client = CLIENTS[(self.rng.next() * CLIENTS.len() as f64) as usize % CLIENTS.len()];
        if !ad {
            self.rng.next();
        }
        Query { name, ad, client }
    }

    /// Moves `dt` seconds on and gives the queries that arrived.
    pub fn step(&mut self, dt: f64) -> Vec<Query> {
        self.clock += dt;
        let mut out = vec![];
        self.next -= dt;
        while self.next <= 0.0 {
            let query = self.generate();
            self.queries += 1;
            self.blocked += u32::from(query.ad && !self.paused);
            out.push(query);
            if self.burst > 0 {
                self.burst -= 1;
                self.next += 0.2 + self.rng.next() * 0.4;
            } else {
                if self.rng.next() < 0.18 {
                    self.burst = 1 + (self.rng.next() * 3.0) as u32;
                }
                self.next += 0.6 - (1.0 - self.rng.next()).ln() * 1.6;
            }
        }
        out
    }

    /// The share of today's queries that were blocked.
    pub fn percent(&self) -> Option<Share> {
        Share::of(u64::from(self.blocked), u64::from(self.queries))
    }

    /// The time of day.
    pub fn time(&self) -> TimeOfDay {
        let minutes = (self.clock / 60.0) as u32;
        TimeOfDay {
            hour: u8::try_from(minutes / 60 % 24).unwrap_or(0),
            minute: u8::try_from(minutes % 60).unwrap_or(0),
        }
    }
}
