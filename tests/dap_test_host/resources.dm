// Resource files the reloading tests edit: lamp_edited.dmi and lamp_third.dmi
// replace lamp.dmi while the world runs.

/obj/lamp
	icon = 'lamp.dmi'
	icon_state = "on"

/obj/lamp_holder

// Made while the world loads, before world/New
var/obj/lamp/early_lamp = new
var/obj/lamp/placed_lamp
var/obj/lamp_holder/lamp_holder

/proc/make_lamps()
	placed_lamp = new
	lamp_holder = new
	lamp_holder.overlays += image('lamp.dmi', "off")
	return "made"

/proc/lamp_states()
	return jointext(icon_states('lamp.dmi'), ",")

/proc/states_of(thing)
	return jointext(icon_states(thing), ",")

/proc/overlay_states()
	var/image/overlay = lamp_holder.overlays[1]
	return states_of(overlay.icon)
