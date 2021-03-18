// for i686-pc-windows-gnu
.intel_syntax noprefix
.global _pre_call_new_hook
.extern _pre_call_new_original
.extern _handle_init_hook

// Pointer to the newly created object's Value is stored in ESI
_pre_call_new_hook:
  PUSH EBX
  PUSH ECX
  PUSH EDX
  PUSH ESI
  call _handle_instruction
  ADD ESP, 0x04
  POP EDX
  POP ECX
  POP EBX

  // Jump to BYOND's default do_instruction.
  MOV EAX, _execute_instruction_original
  JMP EAX
