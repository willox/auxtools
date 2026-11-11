// for i686-pc-windows-gnu
.intel_syntax noprefix
.global _execute_instruction_hook
.extern _execute_instruction_original
.extern _handle_instruction

// EDI = [CURRENT_EXECUTION_CONTEXT]
_execute_instruction_hook:
  // Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  PUSH EAX
  PUSH ECX
  PUSH EDI
  call _handle_instruction
  MOV EDI, EAX
  ADD ESP, 0x04
  POP ECX
  POP EAX

  // Jump to BYOND's default do_instruction.
  MOV EAX, _execute_instruction_original
  JMP EAX
