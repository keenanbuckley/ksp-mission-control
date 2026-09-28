// orbit.ks provides functions for calculating orbital parameters.
//
// Altitudes are above the body's sea level, as kOS reports them; each function
// adds the body radius where the physics needs a radius.

@lazyGlobal off.

// vis viva equation to get the orbital speed at a specified altitude and orbit semimajoraxis.
function visViva {
    parameter orbitingAltitude.
    parameter semiMajorAxis.
    parameter orbitingBody is body.

    local velocitySquared is orbitingBody:mu * ((2/(orbitingAltitude+orbitingBody:radius)) - (1/semiMajorAxis)).
    return sqrt(velocitySquared).
}
