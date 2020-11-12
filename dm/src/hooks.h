#pragma once
#include <stack>
#include <string>

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

static void clean(Value& val) {
    val.type &= 0xFF;
}

struct AuxtoolsException {
    AuxtoolsException(const char* pMessage)
        : message(pMessage)
    {}

    std::string message;
};

extern std::stack<bool> runtime_contexts;

struct RuntimeContext {
    RuntimeContext(bool intercept_exceptions) {
        runtime_contexts.push(intercept_exceptions);
    }

    ~RuntimeContext() {
        runtime_contexts.pop();
    }
    RuntimeContext(const RuntimeContext&) = delete;
    RuntimeContext& operator=(const RuntimeContext&) = delete;
};
