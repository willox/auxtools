#pragma once
#include <stack>
#include <string>

struct AuxtoolsException {
    AuxtoolsException(const char* pMessage) {
        if (pMessage != nullptr) {
            message = pMessage;
        } else {
            message = "<null>";
        }
    }

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
