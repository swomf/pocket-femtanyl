-- pocket-femtanyl keybinds
-- assumes hyprland v0.55+. require it accordingly.

local femtanyl_movement = {
	W = "up",
	A = "left",
	S = "down",
	D = "right",
}

for key, direction in pairs(femtanyl_movement) do
	hl.bind(key, hl.dsp.event("femtanyl:" .. direction .. ":down"), {
		-- release = false
		non_consuming = true,
		transparent = true,
		ignore_mods = true,
	})

	hl.bind(key, hl.dsp.event("femtanyl:" .. direction .. ":up"), {
		release = true,
		non_consuming = true,
		transparent = true,
		ignore_mods = true,
	})
end
