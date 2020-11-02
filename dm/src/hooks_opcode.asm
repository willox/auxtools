.MODEL FLAT, C
.CODE

EXTERN execute_instruction_byond : PTR
handle_custom_instruction PROTO, opcode: DWORD

invalid_instruction_hook PROC PUBLIC
  ; Handle the instruction and put [current_execution_context] into EAX.
  INVOKE handle_custom_instruction, EDX

  ; Continue to next instruction.
  ; ECX and EDX are immediately overwritten so don't worry about them.
  MOV ECX, execute_instruction_byond
  JMP ECX
invalid_instruction_hook ENDP

END