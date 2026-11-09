-- lain-progress.lua: reports playback position for `lain watch`.
--
-- Wired by the CLI, never by hand:
--   mpv --script=lain-progress.lua \
--       --script-opts=lain-state=/path/to/state.json \
--       <stream-url>
--
-- The script writes measured position per playlist entry on pause,
-- every 10 s, and on end/shutdown. The CLI sends it to the server.
local mp = require "mp"
local utils = require "mp.utils"

local opts = { state = "", state_dir = "", resume_file = "", language = "" }
require("mp.options").read_options(opts, "lain")

local aliases = {
    fre = "fra", ger = "deu", chi = "zho", dut = "nld",
    gre = "ell", rum = "ron", cze = "ces", slo = "slk",
    per = "fas", may = "msa", alb = "sqi", arm = "hye",
    baq = "eus", bur = "mya", ice = "isl", mac = "mkd",
}
local function matches(actual)
    local code = string.lower(actual or ""):match("^[a-z]+") or ""
    return code ~= "" and (aliases[code] or code) == (aliases[opts.language] or opts.language)
end

local function choose_tracks()
    if opts.language == "" then return end
    local tracks = mp.get_property_native("track-list") or {}
    local audio, subtitle = nil, nil
    for _, track in ipairs(tracks) do
        if track.type == "audio" and matches(track.lang) and not audio then audio = track.id end
        if track.type == "sub" and matches(track.lang) and not subtitle then subtitle = track.id end
    end
    if audio then
        mp.set_property("aid", tostring(audio))
        mp.set_property("sid", "no")
    elseif subtitle then
        mp.set_property("sid", tostring(subtitle))
    else
        mp.set_property("sid", "no")
    end
end

local resumes = {}
if opts.resume_file ~= "" then
    local f = io.open(opts.resume_file, "r")
    if f then
        resumes = utils.parse_json(f:read("*a")) or {}
        f:close()
    end
end

local current_index = nil
local last_pos = nil
local last_dur = nil

local function write_state(ending)
    local index = ending and current_index or mp.get_property_number("playlist-pos")
    if index == nil then index = current_index end
    if index ~= current_index then
        current_index, last_pos, last_dur = index, nil, nil
    end
    local pos = mp.get_property_number("time-pos") or last_pos
    local dur = mp.get_property_number("duration") or last_dur
    if pos == nil or dur == nil or dur <= 0 then return end
    last_pos, last_dur = pos, dur
    local path = opts.state
    if opts.state_dir ~= "" and index ~= nil then
        path = opts.state_dir .. "/" .. tostring(index) .. ".json"
    end
    if path == "" then return end
    local tmp = path .. ".tmp"
    local f = io.open(tmp, "w")
    if not f then return end
    f:write(utils.format_json({ position_sec = pos, duration_sec = dur }))
    f:close()
    os.rename(tmp, path)
end

mp.register_event("file-loaded", function()
    current_index = mp.get_property_number("playlist-pos")
    last_pos, last_dur = nil, nil
    choose_tracks()
    local at = current_index and resumes[tostring(current_index)]
    if at and at >= 5 then mp.commandv("seek", at, "absolute") end
end)
mp.observe_property("pause", "bool", function() write_state(false) end)
mp.register_event("shutdown", function() write_state(true) end)
mp.register_event("end-file", function(event)
    if event.reason == "eof" and last_dur then last_pos = last_dur end
    write_state(true)
end)
mp.add_periodic_timer(1, write_state)
