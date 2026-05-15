// maneuver_helpers.ks - helpers used by maneuver.ks.

@lazyGlobal off.

function rocketEquationFinalMass {
    parameter startMass.
    parameter dv.
    parameter ev.
    return startMass / (constant:e^(dv/ev)).
}

function burnTime {
    parameter startMass.
    parameter dv.
    parameter ev.
    parameter flowRate.
    local dm is startMass - rocketEquationFinalMass(startMass, dv, ev).
    return dm / flowRate.
}

function meanBurnTime {
    parameter startMass.
    parameter dv.
    parameter ev.
    parameter flowRate.
    return burnTime(startMass, dv/2, ev, flowRate).
}

function availableMassFlowRate {
    local myEngines is list().
    list engines in myEngines.
    local flowRateSum is 0.
    for eng in myEngines {
        if eng:availableThrust > 0 {
            set flowRateSum to flowRateSum + eng:maxMassFlow * eng:thrustLimit / 100.
        }
    }
    return flowRateSum.
}

function availableMassFlowRateAt {
    parameter pressure.
    local myEngines is list().
    list engines in myEngines.
    local flowRateSum is 0.
    for eng in myEngines {
        set flowRateSum to flowRateSum + eng:availableThrustAt(pressure) / (eng:ispAt(pressure) * constant:g0).
    }
    return flowRateSum.
}
