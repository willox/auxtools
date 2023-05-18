// for i686-pc-windows-gnu
.intel_syntax noprefix
.global _execute_instruction_hook
.extern _execute_instruction_original
.extern _handle_instruction

// EAX = [CURRENT_EXECUTION_CONTEXT]
_execute_instruction_hook:
  // Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  PUSH ECX
  PUSH EDX
  PUSH EAX
  call _handle_instruction
  ADD ESP, 0x04
  POP EDX
  POP ECX

  // Jump to BYOND's default do_instruction.
  MOV ECX, _execute_instruction_original
  JMP ECX
