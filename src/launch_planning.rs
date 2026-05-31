use std::sync::Arc;

use anyhow::{Context, Result};
use krpc_client::{services::space_center::SpaceCenter, Client};

/// Pa per kPa. User-facing q params arrive in kPa; the density math is SI.
const KPA_TO_PA: f64 = 1000.0;

/// Unreachable finite stand-in for v_min when there is no atmosphere. Must be
/// finite: serde_json renders non-finite floats as JSON null, which would reach
/// kOS as an undefined vMin. A finite value the vessel never exceeds makes the
/// phase-0 velocity gate simply never fire, so phase 0 exits on altitude alone.
const V_MIN_UNREACHABLE: f64 = 1.0e9;

/// Iterations for the density bisection. ~30 halvings of a sub-100 km range
/// resolves alt_phase2_entry to well under a meter.
const DENSITY_SOLVE_ITERS: u32 = 40;

/// Height (m) added above the sampled local terrain for the phase-0 clearance,
/// covering the launch tower and minor relief between sample points.
const TERRAIN_CLEARANCE_MARGIN: f64 = 150.0;

/// Mission parameters chosen by the operator. Units as noted; q values in kPa.
#[derive(Clone, Copy, Debug)]
pub struct LaunchParams {
    pub final_altitude: f64,        // m
    pub target_inclination: f64,    // deg
    pub target_lan: f64,            // deg, or negative to launch immediately
    pub q_max: f64,                 // kPa
    pub q_auth: f64,                // kPa
    pub q_high: f64,                // kPa
    pub a_max_high: f64,            // deg
    pub a_max_low: f64,             // deg
    pub initial_profile_pitch: f64, // deg
    pub t_ap_target: f64,           // s
    pub throttle_min: f64,          // 0..1
}

impl Default for LaunchParams {
    fn default() -> Self {
        Self {
            final_altitude: 80_000.0,
            target_inclination: 0.0,
            target_lan: -1.0,
            q_max: 20.0,
            q_auth: 0.7,
            q_high: 20.0,
            a_max_high: 10.0,
            a_max_low: 2.0,
            initial_profile_pitch: 85.0,
            t_ap_target: 30.0,
            throttle_min: 0.1,
        }
    }
}

impl LaunchParams {
    /// Reads the user-facing keys out of the command's `args` object, falling
    /// back to the Kerbin-LKO defaults for any key the caller omits.
    pub fn from_args(args: &serde_json::Value) -> Self {
        let d = Self::default();
        let f =
            |key: &str, fallback: f64| args.get(key).and_then(|v| v.as_f64()).unwrap_or(fallback);
        Self {
            final_altitude: f("finalAltitude", d.final_altitude),
            target_inclination: f("targetInclination", d.target_inclination),
            target_lan: f("targetLan", d.target_lan),
            q_max: f("qMax", d.q_max),
            q_auth: f("qAuth", d.q_auth),
            q_high: f("qHigh", d.q_high),
            a_max_high: f("aMaxHigh", d.a_max_high),
            a_max_low: f("aMaxLow", d.a_max_low),
            initial_profile_pitch: f("initialProfilePitch", d.initial_profile_pitch),
            t_ap_target: f("tApTarget", d.t_ap_target),
            throttle_min: f("throttleMin", d.throttle_min),
        }
    }
}

/// Constants computed server-side from mission params plus live body data, then
/// shipped to kOS so the kerboscript runs no startup math.
#[derive(Clone, Copy, Debug)]
pub struct LaunchDerived {
    pub terrain_max: f64,      // m
    pub v_min: f64,            // m/s
    pub alt_phase2_entry: f64, // m
    pub terminal_pitch: f64,   // deg
}

/// Square of the surface-launch energy bound on phase-2 velocity, from vis-viva
/// for an orbit whose apoapsis is `final_alt`: 2 mu (R + h) / (R (2R + h)).
pub fn v_upper_sq(mu: f64, radius: f64, final_alt: f64) -> f64 {
    2.0 * mu * (radius + final_alt) / (radius * (2.0 * radius + final_alt))
}

/// Density at the phase-1 / phase-2 regime split: 2 qMax / v_upper^2, with qMax
/// converted from kPa to Pa so the result is kg/m^3.
pub fn rho_threshold(q_max_kpa: f64, v_upper_sq: f64) -> f64 {
    2.0 * (q_max_kpa * KPA_TO_PA) / v_upper_sq
}

/// Airspeed producing the aerodynamic-authority floor qAuth at the launch-site
/// density: sqrt(2 qAuth / rho_launch), qAuth converted kPa to Pa.
pub fn v_min(q_auth_kpa: f64, rho_launch: f64) -> f64 {
    (2.0 * (q_auth_kpa * KPA_TO_PA) / rho_launch).sqrt()
}

/// Minimum prograde pitch (deg) that holds time-to-apoapsis at `t_ap_target`:
/// vertical until v exceeds g*t, then arcsin(g t / v).
pub fn pitch_min_deg(g: f64, t_ap_target: f64, v: f64) -> f64 {
    if v <= g * t_ap_target {
        90.0
    } else {
        (g * t_ap_target / v).asin().to_degrees()
    }
}

/// Estimated phase-2-entry speed from energy scaling: v^2 ~ v_upper^2 alt / h.
pub fn v_est_at(v_upper_sq: f64, alt_phase2_entry: f64, final_alt: f64) -> f64 {
    (v_upper_sq * alt_phase2_entry / final_alt).sqrt()
}

/// Bisection for the altitude where a monotonic-decreasing density equals
/// `target`, over [lo, hi]. `rho_at` samples density at an altitude. If the
/// target lies outside [rho(hi), rho(lo)] the result clamps to the matching
/// bound, which the caller then reconciles against terrain.
pub fn solve_alt_for_density<F: Fn(f64) -> f64>(
    rho_at: F,
    target: f64,
    lo: f64,
    hi: f64,
    iters: u32,
) -> f64 {
    let (mut a, mut b) = (lo, hi);
    for _ in 0..iters {
        let mid = 0.5 * (a + b);
        if rho_at(mid) > target {
            a = mid;
        } else {
            b = mid;
        }
    }
    0.5 * (a + b)
}

/// Assembles the 14-key config Lexicon kOS reads. qAuth is intentionally absent:
/// it is consumed here to derive v_min and never used kerboscript-side.
pub fn build_launch_payload(p: &LaunchParams, d: &LaunchDerived) -> serde_json::Value {
    serde_json::json!({
        "finalAltitude": p.final_altitude,
        "targetInclination": p.target_inclination,
        "targetLan": p.target_lan,
        "qMax": p.q_max,
        "qHigh": p.q_high,
        "aMaxHigh": p.a_max_high,
        "aMaxLow": p.a_max_low,
        "initialProfilePitch": p.initial_profile_pitch,
        "tApTarget": p.t_ap_target,
        "throttleMin": p.throttle_min,
        "terrainMax": d.terrain_max,
        "vMin": d.v_min,
        "altPhase2Entry": d.alt_phase2_entry,
        "terminalPitch": d.terminal_pitch,
    })
}

/// Computes the launch constants for the active vessel's body. Reads body
/// gravity/atmosphere data and the launch-site position over kRPC, solves the
/// regime-split altitude against KSP's own density curve, and returns the
/// derived values. Several round-trips, run once per launch command.
pub async fn plan_launch(client: &Arc<Client>, p: LaunchParams) -> Result<LaunchDerived> {
    let sc = SpaceCenter::new(client.clone());

    let vessel = sc.get_active_vessel().await.context("get active vessel")?;
    let orbit = vessel.get_orbit().await.context("get vessel orbit")?;
    let body = orbit.get_body().await.context("get orbit body")?;

    let mu = body
        .get_gravitational_parameter()
        .await
        .context("get gravitational parameter")?;
    let radius = body
        .get_equatorial_radius()
        .await
        .context("get equatorial radius")?;
    let has_atm = body
        .get_has_atmosphere()
        .await
        .context("get has atmosphere")?;
    let atm_depth = if has_atm {
        body.get_atmosphere_depth()
            .await
            .context("get atmosphere depth")?
    } else {
        0.0
    };

    // Launch-site position in the body's reference frame.
    let body_frame = body.get_reference_frame().await.context("get body frame")?;
    let flight = vessel
        .flight(Some(&body_frame))
        .await
        .context("get flight in body frame")?;
    let launch_alt = flight
        .get_mean_altitude()
        .await
        .context("get launch altitude")?;
    let launch_lat = flight.get_latitude().await.context("get launch latitude")?;
    let launch_lng = flight
        .get_longitude()
        .await
        .context("get launch longitude")?;

    let terrain_max = sample_terrain_max(&body, launch_lat, launch_lng, launch_alt, radius).await?;

    let v_up2 = v_upper_sq(mu, radius, p.final_altitude);
    let g_surf = mu / (radius * radius);

    let rho_launch = if has_atm {
        body.density_at(launch_alt)
            .await
            .context("get launch-site density")?
    } else {
        0.0
    };
    let v_min_val = if rho_launch > 0.0 {
        v_min(p.q_auth, rho_launch)
    } else {
        V_MIN_UNREACHABLE
    };

    let alt_phase2_entry = if has_atm {
        let rho_thr = rho_threshold(p.q_max, v_up2);
        solve_alt_phase2(&body, rho_thr, terrain_max, atm_depth)
            .await?
            .max(terrain_max)
    } else {
        terrain_max
    };

    let v_est = v_est_at(v_up2, alt_phase2_entry, p.final_altitude);
    let terminal_pitch = pitch_min_deg(g_surf, p.t_ap_target, v_est);

    Ok(LaunchDerived {
        terrain_max,
        v_min: v_min_val,
        alt_phase2_entry,
        terminal_pitch,
    })
}

/// Max terrain height the ascent must clear in phase 0: the max of
/// surface_height sampled on great-circle rings around the launch site, floored
/// at the launch-site altitude and lifted by a clearance margin. Local rather
/// than body-global so a flat coastal pad (KSC eastward is ocean) starts the
/// gravity turn low, in the high-authority low-q window, instead of holding
/// vertical until it clears a distant mountain it never overflies.
async fn sample_terrain_max(
    body: &krpc_client::services::space_center::CelestialBody,
    lat: f64,
    lng: f64,
    launch_alt: f64,
    radius: f64,
) -> Result<f64> {
    let ring_dists = [5_000.0, 15_000.0, 25_000.0];
    let bearings = [0.0, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];
    let mut max_h = body
        .surface_height(lat, lng)
        .await
        .context("sample surface height")?
        .max(launch_alt);
    for &dist in &ring_dists {
        for &bearing in &bearings {
            let (slat, slng) = dest_point(lat, lng, bearing, dist, radius);
            let h = body
                .surface_height(slat, slng)
                .await
                .context("sample surface height")?;
            if h > max_h {
                max_h = h;
            }
        }
    }
    Ok(max_h + TERRAIN_CLEARANCE_MARGIN)
}

/// Great-circle destination: the lat/lng (deg) reached by traveling `dist`
/// meters along `bearing` (deg) from (lat, lng) on a sphere of `radius`.
fn dest_point(lat: f64, lng: f64, bearing: f64, dist: f64, radius: f64) -> (f64, f64) {
    let ang = dist / radius;
    let lat1 = lat.to_radians();
    let lng1 = lng.to_radians();
    let brg = bearing.to_radians();
    let lat2 = (lat1.sin() * ang.cos() + lat1.cos() * ang.sin() * brg.cos()).asin();
    let lng2 =
        lng1 + (brg.sin() * ang.sin() * lat1.cos()).atan2(ang.cos() - lat1.sin() * lat2.sin());
    (lat2.to_degrees(), lng2.to_degrees())
}

/// Bisects KSP's live density curve for the altitude where density equals
/// `target`, over [lo, hi]. Density decreases with altitude, so a sample above
/// the target means the boundary is higher up.
async fn solve_alt_phase2(
    body: &krpc_client::services::space_center::CelestialBody,
    target: f64,
    lo: f64,
    hi: f64,
) -> Result<f64> {
    let (mut a, mut b) = (lo, hi);
    for _ in 0..DENSITY_SOLVE_ITERS {
        let mid = 0.5 * (a + b);
        let rho = body.density_at(mid).await.context("sample density")?;
        if rho > target {
            a = mid;
        } else {
            b = mid;
        }
    }
    Ok(0.5 * (a + b))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Kerbin: mu = 3.5316e12 m^3/s^2, R = 600 km.
    const KERBIN_MU: f64 = 3.5316e12;
    const KERBIN_R: f64 = 600_000.0;

    #[test]
    fn v_upper_kerbin_lko_is_a_few_km_per_s() {
        let v = v_upper_sq(KERBIN_MU, KERBIN_R, 80_000.0).sqrt();
        assert!((2200.0..2600.0).contains(&v), "v_upper = {v}");
    }

    #[test]
    fn rho_threshold_kerbin_lko() {
        let v2 = v_upper_sq(KERBIN_MU, KERBIN_R, 80_000.0);
        let rho = rho_threshold(20.0, v2);
        assert!((rho - 0.0064).abs() < 5.0e-4, "rho_threshold = {rho}");
    }

    #[test]
    fn pitch_min_saturates_to_vertical() {
        // v below g*t has no valid arcsin; command vertical.
        assert_eq!(pitch_min_deg(9.81, 30.0, 100.0), 90.0);
        assert_eq!(pitch_min_deg(9.81, 30.0, 9.81 * 30.0), 90.0);
    }

    #[test]
    fn pitch_min_nominal() {
        let p = pitch_min_deg(9.81, 30.0, 760.0);
        let expect = (9.81 * 30.0 / 760.0_f64).asin().to_degrees();
        assert!((p - expect).abs() < 1.0e-6);
        assert!((20.0..30.0).contains(&p), "pitch_min = {p}");
    }

    #[test]
    fn v_min_from_sea_level_density() {
        let v = v_min(0.7, 1.225);
        assert!((v - 33.8).abs() < 0.5, "v_min = {v}");
    }

    #[test]
    fn v_est_is_below_upper_bound_and_monotonic() {
        let v2 = v_upper_sq(KERBIN_MU, KERBIN_R, 80_000.0);
        let lo = v_est_at(v2, 10_000.0, 80_000.0);
        let hi = v_est_at(v2, 30_000.0, 80_000.0);
        assert!(lo < hi);
        assert!(hi < v2.sqrt());
    }

    #[test]
    fn density_solve_matches_exponential_closed_form() {
        // Synthetic isothermal atmosphere: rho(h) = rho0 exp(-h/H).
        let rho0 = 1.225;
        let scale_h = 5600.0;
        let target = 0.0064;
        let solved =
            solve_alt_for_density(|h| rho0 * (-h / scale_h).exp(), target, 0.0, 70_000.0, 60);
        let closed_form = scale_h * (rho0 / target).ln();
        assert!(
            (solved - closed_form).abs() < 50.0,
            "solved {solved} vs {closed_form}"
        );
    }

    #[test]
    fn density_solve_clamps_when_target_out_of_range() {
        let rho0 = 1.225;
        let scale_h = 5600.0;
        let rho_at = |h: f64| rho0 * (-h / scale_h).exp();
        // Target denser than anything in range -> clamps toward lo.
        let below = solve_alt_for_density(rho_at, 2.0, 0.0, 70_000.0, 40);
        assert!(below < 1000.0, "expected near lo, got {below}");
        // Target thinner than anything in range -> clamps toward hi.
        let above = solve_alt_for_density(rho_at, 1.0e-9, 0.0, 70_000.0, 40);
        assert!(above > 69_000.0, "expected near hi, got {above}");
    }

    #[test]
    fn dest_point_moves_the_expected_distance_and_direction() {
        // Due north from the equator: latitude rises by dist/radius radians, lng unchanged.
        let (lat, lng) = dest_point(0.0, 0.0, 0.0, 60_000.0, KERBIN_R);
        assert!((lat - (60_000.0_f64 / KERBIN_R).to_degrees()).abs() < 1.0e-6);
        assert!(lng.abs() < 1.0e-6);
        // Due east from the equator: lng rises by dist/radius radians, lat ~ unchanged.
        let (lat_e, lng_e) = dest_point(0.0, 0.0, 90.0, 60_000.0, KERBIN_R);
        assert!(lat_e.abs() < 1.0e-6);
        assert!((lng_e - (60_000.0_f64 / KERBIN_R).to_degrees()).abs() < 1.0e-3);
    }

    #[test]
    fn from_args_uses_defaults_for_missing_keys() {
        let args = serde_json::json!({ "finalAltitude": 120_000.0, "qMax": 35.0 });
        let p = LaunchParams::from_args(&args);
        assert_eq!(p.final_altitude, 120_000.0);
        assert_eq!(p.q_max, 35.0);
        assert_eq!(p.t_ap_target, 30.0); // default
        assert_eq!(p.initial_profile_pitch, 85.0); // default
    }

    #[test]
    fn payload_has_derived_keys_and_omits_q_auth() {
        let p = LaunchParams::default();
        let d = LaunchDerived {
            terrain_max: 6800.0,
            v_min: 33.8,
            alt_phase2_entry: 29_500.0,
            terminal_pitch: 22.0,
        };
        let payload = build_launch_payload(&p, &d);
        assert!(payload.get("qAuth").is_none());
        assert_eq!(payload["terrainMax"], 6800.0);
        assert_eq!(payload["altPhase2Entry"], 29_500.0);
        assert_eq!(payload["qMax"], 20.0);
    }
}
