.MODEL FLAT, C
.CODE

EXTERN execute_instruction_original : PTR
handle_instruction PROTO, opcode: DWORD

; EAX = [CURRENT_EXECUTION_CONTEXT]
execute_instruction_hook PROC PUBLIC
  ; Give rust a chance to handle the instruction. Leaves [CURRENT_EXECUTION_CONTEXT] in EAX.
  PUSH ECX
  PUSH EDX
  INVOKE handle_instruction, EAX
  POP EDX
  POP ECX

  ; Jump to BYOND's default do_instruction.
  MOV ECX, execute_instruction_original
  JMP ECX
execute_instruction_hook ENDP

END