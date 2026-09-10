-- lain-progress.lua: reports playback position for `lain watch`.
--
-- Wired by the CLI, never by hand:
--   mpv --script=lain-progress.lua \
--       --script-opts=lain-state=/path/to/state.json \
--       <stream-url>
--
-- The script only writes a small JSON file (position + duration) on
-- pause, every 10 s, and on shutdown/end-of-file. The CLI reads it
-- after mpv exits and PUTs progress to the server. No network from
-- lua, no credentials in the player process environment.
local mp = require "mp"
local utils = require "mp.utils"

local opts = { state = "" }
require("mp.options").read_options(opts, "lain")

local function write_state()
    if opts.state == "" then
        return
    end
    local pos = mp.get_property_number("time-pos")
    local dur = mp.get_property_number("duration")
    if pos == nil or dur == nil or dur <= 0 then
        return
    end
    local f = io.open(opts.state, "w")
    if f == nil then
        return
    end
    f:write(utils.format_json({ position_sec = pos, duration_sec = dur }))
    f:close()
end

mp.observe_property("pause", "bool", write_state)
mp.register_event("shutdown", write_state)
mp.register_event("end-file", write_state)
mp.add_periodic_timer(10, write_state)
