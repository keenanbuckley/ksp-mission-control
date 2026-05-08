// ag_listener.ks - mc-link boot script.
//
// On boot: claim the "mc" part tag if no other holder exists, then drain
// the kIPC inbox and dispatch action-group toggles. Messages arrive as
// kerboscript Lexicons (kIPC has already deserialized the JSON envelope).

@lazyGlobal off.

print "ag_listener: waiting for ship to unpack.".
wait until ship:unpacked.

local MC_TAG       is "mc".
local PROTO_VER    is 1.
local PASSIVE_POLL is 5.   // seconds between vessel-change re-checks

function otherMcHolderExists {
    local self_uid is core:part:uid.
    for p in ship:parts {
        if p:tag = MC_TAG and p:uid <> self_uid {
            return true.
        }
    }
    return false.
}

function claimMc {
    set core:part:tag to MC_TAG.
    print "ag_listener: claimed mc tag on " + core:part:name + ".".
}

function toggleAg {
    parameter n.
    if      n = 1  { toggle ag1.  }
    else if n = 2  { toggle ag2.  }
    else if n = 3  { toggle ag3.  }
    else if n = 4  { toggle ag4.  }
    else if n = 5  { toggle ag5.  }
    else if n = 6  { toggle ag6.  }
    else if n = 7  { toggle ag7.  }
    else if n = 8  { toggle ag8.  }
    else if n = 9  { toggle ag9.  }
    else if n = 10 { toggle ag10. }
    else {
        print "ag_listener: ag n out of range: " + n + ".".
        return.
    }
    print "ag_listener: toggled AG" + n + ".".
}

function handleMessage {
    parameter content.
    if not content:istype("Lexicon") {
        print "ag_listener: bad message type; dropping.".
        return.
    }
    if not content:haskey("protocol_version")
       or content:protocol_version <> PROTO_VER {
        print "ag_listener: missing/bad protocol_version; dropping.".
        return.
    }
    if not content:haskey("op") {
        print "ag_listener: no op field; dropping.".
        return.
    }
    local op is content:op.
    if op = "toggle_ag" {
        if not content:haskey("n") {
            print "ag_listener: toggle_ag missing n; dropping.".
            return.
        }
        toggleAg(content:n).
    } else {
        print "ag_listener: unknown op '" + op + "'; dropping.".
    }
}

function runActive {
    print "ag_listener: active mode.".
    until false {
        wait until not core:messages:empty.
        until core:messages:empty {
            handleMessage(core:messages:pop():content).
        }
    }
}

function runPassive {
    print "ag_listener: passive mode (mc held elsewhere).".
    until false {
        wait PASSIVE_POLL.
        if not otherMcHolderExists() {
            print "ag_listener: no mc holder found; promoting.".
            claimMc().
            runActive().
        }
    }
}

if otherMcHolderExists() {
    runPassive().
} else {
    claimMc().
    runActive().
}
