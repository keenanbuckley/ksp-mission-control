use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use krpc_client::{services::space_center::SpaceCenter, Client};

#[derive(Clone, Copy, Debug)]
pub struct CircPlan {
    pub dv: f64,
    pub ut: f64,
}

pub fn circularization_dv(mu: f64, r_a: f64, a: f64) -> f64 {
    (mu / r_a).sqrt() - (mu * (2.0 / r_a - 1.0 / a)).sqrt()
}

pub async fn plan_circ(client: &Arc<Client>) -> Result<CircPlan> {
    let sc = SpaceCenter::new(client.clone());

    let vessel = sc.get_active_vessel().await.context("get active vessel")?;
    let orbit = vessel.get_orbit().await.context("get vessel orbit")?;

    let body = orbit.get_body().await.context("get orbit body")?;
    let mu = body
        .get_gravitational_parameter()
        .await
        .context("get gravitational parameter")?;
    let r_a = orbit.get_apoapsis().await.context("get apoapsis")?;
    let a = orbit
        .get_semi_major_axis()
        .await
        .context("get semi-major axis")?;
    let time_to_apo = orbit
        .get_time_to_apoapsis()
        .await
        .context("get time to apoapsis")?;
    let ut_now = sc.get_ut().await.context("get ut")?;

    if a <= 0.0 || !r_a.is_finite() {
        return Err(anyhow!(
            "non-elliptic orbit (a={a}, apoapsis={r_a}); refusing to plan"
        ));
    }

    Ok(CircPlan {
        dv: circularization_dv(mu, r_a, a),
        ut: ut_now + time_to_apo,
    })
}
