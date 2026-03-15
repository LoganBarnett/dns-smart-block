//! DNS resolution check for domain validation.
//!
//! This module contains a DNS-based NXDOMAIN guard that was originally used
//! in the queue-processor to detect non-existent domains before spending LLM
//! resources on them.  It is currently **disabled** because performing a DNS
//! lookup from inside the queue-processor routes through the local DNS stack
//! (Blocky, or whatever the system resolver points at), generating a new log
//! entry that the log-processor then enqueues — effectively doubling every
//! domain processed.  With multiple queue-processor workers the problem
//! compounds further: each worker fires its own lookup before any
//! classification is written, and the resulting log entries cascade until the
//! first classification lands.
//!
//! There is no clean way to "colour" a DNS query so that the log-processor
//! can recognise and ignore it: per-client filtering in Blocky relies on
//! source IP, and making the queue-processor use a distinct loopback address
//! for that purpose is fragile and host-specific.  Using a reverse PTR lookup
//! on the already-resolved IP (which would produce a different
//! `question_name`) avoids re-queuing the same domain name, but PTR records
//! are too unreliable in practice (CDN IPs return their own infrastructure
//! hostnames, not the origin domain) to be useful for validation.
//!
//! In practice the `response_type=RESOLVED` filter applied by the
//! log-processor already excludes NXDOMAIN entries: Blocky only emits
//! `RESOLVED` for queries that returned a real address.  So the guard is
//! redundant for the current log-tailing architecture (Mode A).
//!
//! Roadmap context: Mode B (DNS proxy) and Mode C (standalone DNS server)
//! would put dns-smart-block directly in the resolution path.  At that point
//! the NXDOMAIN signal is available from the DNS response object itself with
//! no re-resolution required, and this code could be revived.  For the
//! foreseeable future it remains unused.

#![allow(dead_code)]

use hickory_resolver::TokioAsyncResolver;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::error::ResolveErrorKind;
use hickory_resolver::proto::op::ResponseCode;
use tracing::warn;

pub enum DnsOutcome {
  /// Domain has at least one address record — proceed to LLM classification.
  Resolves,
  /// Authoritative NXDOMAIN — domain does not exist.
  Nxdomain,
  /// Transient failure (SERVFAIL, timeout, …) — skip this message and let it
  /// be retried naturally on the next log entry for the domain.
  TransientError(String),
}

pub async fn dns_check(
  resolver: &TokioAsyncResolver,
  domain: &str,
) -> DnsOutcome {
  match resolver.lookup_ip(domain).await {
    Ok(_) => DnsOutcome::Resolves,
    Err(e) => match e.kind() {
      ResolveErrorKind::NoRecordsFound { response_code, .. }
        if *response_code == ResponseCode::NXDomain =>
      {
        DnsOutcome::Nxdomain
      }
      // Any other NoRecordsFound (e.g. NODATA — domain exists but has no
      // A/AAAA records) is treated as "resolves" so we attempt LLM
      // classification rather than silently discarding the domain.
      ResolveErrorKind::NoRecordsFound { .. } => DnsOutcome::Resolves,
      _ => DnsOutcome::TransientError(e.to_string()),
    },
  }
}

/// Build a DNS resolver from the system configuration, falling back to
/// public resolvers if `/etc/resolv.conf` cannot be read.
pub fn build_resolver() -> TokioAsyncResolver {
  match hickory_resolver::system_conf::read_system_conf() {
    Ok((config, opts)) => TokioAsyncResolver::tokio(config, opts),
    Err(e) => {
      warn!(
        "Could not read system DNS config ({}), falling back to defaults",
        e
      );
      TokioAsyncResolver::tokio(
        ResolverConfig::default(),
        ResolverOpts::default(),
      )
    }
  }
}
