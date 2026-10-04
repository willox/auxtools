// for i686-unknown-linux-gnu
.intel_syntax noprefix
.global execute_instruction_hook
.extern execute_instruction_original
.extern handle_instruction

// ESI = [CURRENT_EXECUTION_CONTEXT]
execute_instruction_hook:
  // Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  // The 16 bytes pushed here keep ESP 16-byte aligned at the call.
  SUB ESP, 0x04
  PUSH ECX
  PUSH EDX
  PUSH ESI
  call handle_instruction
  MOV ESI, EAX
  ADD ESP, 0x04
  POP EDX
  POP ECX
  ADD ESP, 0x04

  // Jump to BYOND's default do_instruction.
  MOV EAX, execute_instruction_original
  JMP EAX
