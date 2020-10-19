#include <stdint.h>

#ifdef _WIN32
#define LINUX_REGPARM2
#define LINUX_REGPARM3
#else
#define LINUX_REGPARM2 __attribute__((regparm(2)))
#define LINUX_REGPARM3 __attribute__((regparm(3)))
#endif

struct Value {
	uint32_t type;
	uint32_t value;
};

//
// BYOND likes to use C++ exceptions for some stuff (like runtimes) - Rust can't catch them and code will just unroll back to before our hooks
// We use these wrappers to hackily handle that and let Rust know an exception happened instead of letting it propogate
//

#define DEFINE_byond(name, ret_type, params) \
    using Fn##name##_byond = ret_type(*)params; \
    extern "C" Fn##name##_byond name##_byond = nullptr;

#define DEFINE_byond_REGPARM2(name, ret_type, params) \
    using Fn##name##_byond = ret_type(LINUX_REGPARM2 *)params; \
    extern "C" Fn##name##_byond name##_byond = nullptr;

#define DEFINE_byond_REGPARM3(name, ret_type, params) \
    using Fn##name##_byond = ret_type(LINUX_REGPARM3 *)params; \
    extern "C" Fn##name##_byond name##_byond = nullptr;

DEFINE_byond_REGPARM3(call_proc_by_id, Value, (Value, uint32_t, uint32_t, uint32_t, Value, const Value*, uint32_t, uint32_t, uint32_t))
DEFINE_byond_REGPARM3(call_datum_proc_by_name, Value, (Value, uint32_t, uint32_t, Value, const Value*, uint32_t, uint32_t, uint32_t))
DEFINE_byond(get_proc_array_entry, void*, (uint32_t))
DEFINE_byond_REGPARM3(get_string_id, uint32_t, (const char*, bool, bool, bool))
DEFINE_byond(get_variable, Value, (Value, uint32_t))
DEFINE_byond(set_variable, void, (Value, uint32_t, Value))
DEFINE_byond(get_string_table_entry, void*, (uint32_t))
DEFINE_byond(inc_ref_count, void, (Value))
DEFINE_byond(dec_ref_count, void, (Value))
DEFINE_byond_REGPARM3(get_list_by_id, void*, (uint32_t))
DEFINE_byond_REGPARM3(get_assoc_element, Value, (Value, Value))
DEFINE_byond_REGPARM3(set_assoc_element, void, (Value, Value, Value))
DEFINE_byond(create_list, uint32_t, (uint32_t))
DEFINE_byond_REGPARM2(append_to_list, void, (Value, Value))
DEFINE_byond_REGPARM2(remove_from_list, void, (Value, Value))
DEFINE_byond(get_length, uint32_t, (Value))

extern "C" uint8_t call_proc_by_id(
    Value* out,
    Value usr,
	uint32_t proc_type,
	uint32_t proc_id,
	uint32_t unk_0,
	Value src,
	const Value* args,
	uint8_t args_count,
	uint32_t unk_1,
	uint32_t unk_2
) {
    try {
        *out = call_proc_by_id_byond(usr, proc_type, proc_id, unk_0, src, args, args_count, unk_1, unk_2);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t call_datum_proc_by_name(
    Value* out,
    Value usr,
	uint32_t proc_type,
	uint32_t proc_name,
	Value src,
	const Value* args,
	uint8_t args_count,
	uint32_t unk_0,
	uint32_t unk_1
) {
    try {
        *out = call_datum_proc_by_name_byond(usr, proc_type, proc_name, src, args, args_count, unk_0, unk_1);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t get_proc_array_entry(void** out, uint32_t id) {
    try {
        *out = get_proc_array_entry_byond(id);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t get_string_id(uint32_t* out, const char* data, bool a, bool b, bool c) {
    try {
        *out = get_string_id_byond(data, a, b, c);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t get_variable(Value* out, Value datum, uint32_t string_id) {
    try {
        *out = get_variable_byond(datum, string_id);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t set_variable(Value datum, uint32_t string_id, Value value) {
    try {
        set_variable_byond(datum, string_id, value);
        return 1;
    } catch(...) {
        return 0;
    }  
}

extern "C" uint8_t get_string_table_entry(void** out, uint32_t string_id) {
    try {
        *out = get_string_table_entry_byond(string_id);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t inc_ref_count(Value value) {
    try {
        inc_ref_count_byond(value);
        return 1;
    } catch(...) {
        return 0;
    }  
}

extern "C" uint8_t dec_ref_count(Value value) {
    try {
        dec_ref_count_byond(value);
        return 1;
    } catch(...) {
        return 0;
    }  
}

extern "C" uint8_t get_list_by_id(void** out, uint32_t list_id) {
    try {
        *out = get_list_by_id_byond(list_id);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t get_assoc_element(Value* out, Value datum, Value index) {
    try {
        *out = get_assoc_element_byond(datum, index);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t set_assoc_element(Value datum, Value index, Value value) {
    try {
        set_assoc_element_byond(datum, index, value);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t create_list(uint32_t* out, uint32_t reserve_capacity) {
    try {
        *out = create_list_byond(reserve_capacity);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t append_to_list(Value list, Value value) {
    try {
        append_to_list_byond(list, value);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t remove_from_list(Value list, Value value) {
    try {
        remove_from_list_byond(list, value);
        return 1;
    } catch(...) {
        return 0;
    }
}

extern "C" uint8_t get_length(uint32_t* out, Value value) {
    try {
        *out = get_length_byond(value);
        return 1;
    } catch(...) {
        return 0;
    }
}
