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

local opts = {
    state = "", state_dir = "", resume_file = "",
    -- D-280: the account's track preferences. `language` is D-071's one
    -- language, read as both when the other two are empty.
    audio_language = "", subtitle_language = "", subtitle_mode = "", language = "",
}
require("mp.options").read_options(opts, "lain")
if opts.audio_language == "" and opts.subtitle_language == "" and opts.language ~= "" then
    opts.audio_language, opts.subtitle_language = opts.language, opts.language
end
if opts.subtitle_mode == "" then opts.subtitle_mode = "auto" end

local aliases = {
    fre = "fra", ger = "deu", chi = "zho", dut = "nld",
    gre = "ell", rum = "ron", cze = "ces", slo = "slk",
    per = "fas", may = "msa", alb = "sqi", arm = "hye",
    baq = "eus", bur = "mya", ice = "isl", mac = "mkd",
}
local function code(lang)
    local c = string.lower(lang or ""):match("^[a-z]+") or ""
    return aliases[c] or c
end
local function same(actual, preferred)
    local a = code(actual)
    return a ~= "" and preferred ~= "" and a == code(preferred)
end

-- The rule of lain-watch's tracks.rs (D-280), on mpv's track list.
local function choose_tracks()
    local a_lang, s_lang, mode = opts.audio_language, opts.subtitle_language, opts.subtitle_mode
    if a_lang == "" and s_lang == "" and mode == "auto" then return end
    local tracks = mp.get_property_native("track-list") or {}
    local audios, subs, tagged = {}, {}, false
    for _, t in ipairs(tracks) do
        if t.type == "audio" then table.insert(audios, t) end
        if t.type == "sub" then table.insert(subs, t) end
        if (t.type == "audio" or t.type == "sub") and code(t.lang) ~= "" and code(t.lang) ~= "und" then tagged = true end
    end
    local audio = nil
    for _, t in ipairs(audios) do
        if same(t.lang, a_lang) then audio = t break end
    end
    if audio then mp.set_property("aid", tostring(audio.id)) end
    if not audio then
        for _, t in ipairs(audios) do
            if t.default then audio = t break end
        end
        audio = audio or audios[1]
    end
    local heard = audio and audio.lang or ""
    local function in_language(lang)
        for _, t in ipairs(subs) do
            if not t.forced and same(t.lang, lang) then return t end
        end
        for _, t in ipairs(subs) do
            if same(t.lang, lang) then return t end
        end
        return nil
    end
    local pick, off = nil, false
    if mode == "off" then
        off = true
    elseif mode == "forced" then
        for _, want in ipairs({ s_lang, heard }) do
            for _, t in ipairs(subs) do
                if not pick and t.forced and same(t.lang, want) then pick = t end
            end
        end
        for _, t in ipairs(subs) do
            if not pick and t.forced and (code(t.lang) == "" or code(t.lang) == "und") then pick = t end
        end
        off = pick == nil
    elseif mode == "always" then
        if s_lang == "" then
            for _, t in ipairs(subs) do
                if not pick and not t.forced and t.default then pick = t end
            end
            for _, t in ipairs(subs) do
                if not pick and not t.forced then pick = t end
            end
            off = pick == nil
        else
            pick = in_language(s_lang)
            off = pick == nil and tagged
        end
    else
        if s_lang == "" or not tagged then return end
        if same(heard, s_lang) then
            off = true
        else
            pick = in_language(s_lang)
            off = pick == nil
        end
    end
    if pick then
        mp.set_property("sid", tostring(pick.id))
    elseif off then
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
