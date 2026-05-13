// dispatch_listener.ks - mc-link boot script.
//
// On boot: claim the "mc" part tag if no other holder exists, then drain
// the kIPC inbox and dispatch ops. Messages arrive as kerboscript Lexicons
// (kIPC has already deserialized the JSON envelope).

@lazyGlobal off.

print "dispatch_listener: waiting for ship to unpack.".
wait until ship:unpacked.

// archive is the working volume. scripts and helpers live there per the
// deploy tool's contract, so absolute paths like "/launch.ks" resolve.
switch to 0.

local MC_TAG       is "mc".
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
    print "dispatch_listener: claimed mc tag on " + core:part:name + ".".
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
        print "dispatch_listener: ag n out of range: " + n + ".".
        return.
    }
    print "dispatch_listener: toggled AG" + n + ".".
}

function handleMessage {
    parameter content.
    if not content:istype("Lexicon") {
        print "dispatch_listener: bad message type; dropping.".
        return.
    }
    if not content:haskey("op") {
        print "dispatch_listener: no op field; dropping.".
        return.
    }
    local op is content:op.
    if op = "toggle_ag" {
        if not content:haskey("n") {
            print "dispatch_listener: toggle_ag missing n; dropping.".
            return.
        }
        toggleAg(content:n).
    } else if op = "run_script" {
        if not content:haskey("path") {
            print "dispatch_listener: run_script missing path; dropping.".
            return.
        }
        if not content:haskey("args") {
            print "dispatch_listener: run_script missing args; dropping.".
            return.
        }
        local p is content:path.
        local a is content:args.
        if not exists("/" + p) {
            print "dispatch_listener: script not found: /" + p + "; dropping.".
            return.
        }
        if not a:istype("List") or a:length <> 6 {
            print "dispatch_listener: run_script args must be a 6-element list; dropping.".
            return.
        }
        print "dispatch_listener: running /" + p + ".".
        // Fixed arity 6 matches launch.ks's signature. Kerboscript has no splat.
        runPath("/" + p, a[0], a[1], a[2], a[3], a[4], a[5]).
        print "dispatch_listener: /" + p + " returned.".
    } else {
        print "dispatch_listener: unknown op '" + op + "'; dropping.".
    }
}

function runActive {
    print "dispatch_listener: active mode.".
    until false {
        wait until not core:messages:empty.
        until core:messages:empty {
            handleMessage(core:messages:pop():content).
        }
    }
}

function runPassive {
    print "dispatch_listener: passive mode (mc held elsewhere).".
    until false {
        wait PASSIVE_POLL.
        if not otherMcHolderExists() {
            print "dispatch_listener: no mc holder found; promoting.".
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
