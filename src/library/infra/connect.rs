//! Explicit connector alignment. No implicit reduction or mixing of utility systems.
use super::{
    geometry::{self, Built},
    Item, Placement,
};

pub fn attach(
    item: &Item,
    p: &Placement,
    own: &str,
    target: &Built,
    target_port: &str,
) -> Result<Placement, String> {
    if let Some(edge) = target_port.strip_prefix("edge-") {
        if item.system != "curb" || own != "in" {
            return Err("vlakrand kan alleen met de in-poort van een band worden verbonden".into());
        }
        let k: usize = edge.parse().map_err(|_| "ongeldige randindex")?;
        let n = target.boundary.len();
        if n < 3 || k >= n {
            return Err("vlakrand bestaat niet".into());
        }
        let a = target.boundary[k];
        let b = target.boundary[(k + 1) % n];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = dx.hypot(dy);
        let signed = (0..n)
            .map(|j| {
                let p = target.boundary[j];
                let q = target.boundary[(j + 1) % n];
                p[0] * q[1] - q[0] * p[1]
            })
            .sum::<f64>()
            .signum();
        let half = item.value(p, "width") / 2.;
        let mut out = p.clone();
        out.at = [
            a[0] + signed * dy / len * half,
            a[1] - signed * dx / len * half,
            a[2],
        ];
        out.rotation = dy.atan2(dx);
        out.path = vec![[0., 0.], [len, 0.]];
        out.elevation_known = target.elevation_known;
        return Ok(out);
    }
    let a = target
        .ports
        .iter()
        .find(|v| v.id == target_port)
        .ok_or("doelaansluitpunt bestaat niet")?;
    let local = geometry::build(
        item,
        &Placement {
            at: [0.; 3],
            rotation: 0.,
            ..p.clone()
        },
    )?;
    let b = local
        .ports
        .iter()
        .find(|v| v.id == own)
        .ok_or("eigen aansluitpunt bestaat niet")?;
    if (a.id == "in" && b.id == "in") || (a.id == "out" && b.id == "out") {
        return Err("aansluitrichtingen botsen: gebruik een ingaande en uitgaande poort".into());
    }
    if a.system.is_empty() || a.system != b.system {
        return Err(format!(
            "aansluitsysteem verschilt: {} / {}",
            a.system, b.system
        ));
    }
    if (a.diameter - b.diameter).abs() > 0.001 {
        return Err(format!(
            "diameter {} / {} mm: kies een passende reducer",
            a.diameter, b.diameter
        ));
    }
    let rotation = (-a.direction[1]).atan2(-a.direction[0]) - b.direction[1].atan2(b.direction[0]);
    let (s, c) = rotation.sin_cos();
    let mut out = p.clone();
    out.rotation = rotation;
    out.elevation_known = target.elevation_known;
    out.at = [
        a.at[0] - c * b.at[0] + s * b.at[1],
        a.at[1] - s * b.at[0] - c * b.at[1],
        a.at[2] - b.at[2],
    ];
    Ok(out)
}

/// Re-resolve child placements topologically; invalid edits never alter the input network.
pub fn resolve(instances: &[Placement]) -> Result<Vec<Placement>, String> {
    let mut pending = instances.to_vec();
    let mut ready: Vec<Placement> = vec![];
    let mut ids = std::collections::HashSet::new();
    for p in &pending {
        if p.instance.is_empty() || !ids.insert(p.instance.clone()) {
            return Err("dubbel of leeg instantie-ID".into());
        }
    }
    while !pending.is_empty() {
        let before = pending.len();
        let mut next = vec![];
        for mut p in pending {
            if let Some(link) = p.link.clone() {
                if let Some(parent) = ready.iter().find(|v| v.instance == link.instance) {
                    if parent.status != p.status {
                        return Err("verbinding tussen verschillende tekenstatussen".into());
                    }
                    let target = geometry::build(super::find(&parent.item)?, parent)?;
                    p = attach(
                        super::find(&p.item)?,
                        &p,
                        &link.own_port,
                        &target,
                        &link.port,
                    )?;
                } else {
                    next.push(p);
                    continue;
                }
            }
            geometry::build(super::find(&p.item)?, &p)?;
            ready.push(p);
        }
        if next.len() == before {
            return Err("kringverbinding of ontbrekend bovenliggend object".into());
        }
        pending = next;
    }
    // A physical socket can have at most one attached child.
    let mut occupied = std::collections::HashSet::new();
    for p in &ready {
        if let Some(l) = &p.link {
            if !occupied.insert((l.instance.clone(), l.port.clone()))
                || !occupied.insert((p.instance.clone(), l.own_port.clone()))
            {
                return Err("aansluitpunt is al bezet".into());
            }
        }
    }
    Ok(ready)
}
