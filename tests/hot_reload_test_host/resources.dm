// Resource files edited individually, together and with shared content
// by the hot reload tests.



/obj/lamp
	icon = 'lamp.dmi'
	icon_state = "on"
	mouse_over_pointer = 'lamp.dmi'

/obj/lamp_holder

/datum/player
	var/sound_file = 'beep.wav'

/obj/crate
	icon = 'crate.dmi'

var/obj/lamp/placed_lamp
var/obj/lamp_holder/lamp_holder
var/obj/crate/placed_crate

// Retain a built resource and create an instance from it after reload.

var/held_lamp_icon = 'lamp.dmi'
var/obj/held_lamp

/proc/make_lamps()
	placed_lamp = new
	lamp_holder = new
	lamp_holder.overlays += image('lamp.dmi', "off")
	return "made"

/proc/make_crate()
	placed_crate = new
	held_lamp = new
	held_lamp.icon = held_lamp_icon
	return "made"

// Compiled proc referencing a resource through a constant.
/proc/lamp_states()
	return jointext(icon_states('lamp.dmi'), ",")

/proc/states_of(thing)
	return jointext(icon_states(thing), ",")

/proc/new_lamp_states()
	var/obj/lamp/lamp = new
	return states_of(lamp.icon)

/proc/overlay_states()
	var/image/overlay = lamp_holder.overlays[1]
	return states_of(overlay.icon)

/proc/beep_ref()
	return "\ref['beep.wav']"

/proc/default_sound_ref()
	var/datum/player/player = new
	return "\ref[player.sound_file]"

/proc/lamp_ref()
	return "\ref['lamp.dmi']"

/proc/crate_ref()
	return "\ref['crate.dmi']"

// After 65,535 appearances, BYOND inserts a blank node for its reserved ID.
var/list/many_appearances

/proc/make_many_appearances()
	many_appearances = list()
	for(var/i in 1 to 66000)
		var/obj/thing = new
		thing.name = "thing [i]"
		many_appearances += thing
	return "made"
