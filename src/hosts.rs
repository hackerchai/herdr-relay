//! Hosts this device has paired with and can access.
use crate::{Error, Result, names::HostName};
use std::path::Path;

/// Paired hosts with their saved names, in pairing order.
pub fn list(profile: &Path, origin: &str) -> Result<Vec<HostName>> {
    crate::pairing::Store::open(profile, origin)?
        .grants()?
        .accessible_hosts
        .iter()
        .map(|g| crate::names::remember(profile, origin, &g.device_id, None, None))
        .collect()
}

/// Forget a host chosen by alias, name or device ID: its access grant, saved
/// name and direct-connection cache. The host keeps its own grant until it
/// revokes this device; pairing again needs a new invitation.
pub fn remove(profile: &Path, origin: &str, selector: &str) -> Result<HostName> {
    let hosts = list(profile, origin)?;
    let matches =
        |pick: &dyn Fn(&HostName) -> bool| hosts.iter().filter(|h| pick(h)).collect::<Vec<_>>();
    let mut found = matches(&|h| h.identity == selector || h.alias == selector);
    if found.is_empty() {
        found = matches(&|h| h.name == selector);
    }
    let [host] = found.as_slice() else {
        return Err(Error::new(
            "HOST_UNKNOWN",
            if found.is_empty() {
                "no paired host matches; use an alias or device ID from hosts list"
            } else {
                "several hosts share that name; use the alias or device ID"
            },
        ));
    };
    let host = (*host).clone();
    crate::pairing::Store::open(profile, origin)?.forget_host(&host.identity)?;
    crate::names::forget(profile, origin, &host.identity)?;
    crate::direct::forget(profile, origin, &host.identity);
    Ok(host)
}
