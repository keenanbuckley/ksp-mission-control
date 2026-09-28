// node.ks provides functions for planning maneuver nodes.

@lazyGlobal off.

runOncePath("/lib/orbit.ks").

// compute orbital speed at apoapsis using apoapsis and periapsis
function velocityApoapsis {
    parameter targetApoapsis.
    parameter targetPeriapsis.
    parameter orbitingBody is body.

    return visViva(targetApoapsis, (targetApoapsis+targetPeriapsis+(2*orbitingBody:radius))/2, orbitingBody).
}

// generate a node at apoapsis to change the height of the periapsis.
// Circularizing at apoapsis is nodeChangePeriapsis(apoapsis).
// Returns -1 when no valid node exists.
function nodeChangePeriapsis {
    parameter targetPeriapsis.
    parameter initialOrbit is orbit.
    parameter safety is true.

    if initialOrbit:eccentricity >= 1 { return -1. }

    // bound target periapsis to range. soiRadius is measured from the body's
    // center, so the altitude needs the body radius added before comparing.
    local orbitBody is initialOrbit:body.
    local inRange is targetPeriapsis > 0 and targetPeriapsis + orbitBody:radius < orbitBody:soiRadius.
    if not initialOrbit:hasNextPatch and (not safety or inRange) {
        local currVel is velocityApoapsis(initialOrbit:apoapsis, initialOrbit:periapsis, orbitBody).
        local targetVel is velocityApoapsis(initialOrbit:apoapsis, targetPeriapsis, orbitBody).
        local deltaV is targetVel - currVel.
        return node(initialOrbit:eta:apoapsis + time:seconds, 0, 0, deltaV).
    }
    return -1.
}
