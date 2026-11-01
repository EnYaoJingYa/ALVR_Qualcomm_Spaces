"""Run parsed feedback through the DLL's frequency and window-state instructions."""
import struct
from pathlib import Path

from replay import BASE, FRAME, STATE, UC_HOOK_CODE, machine, reg


def main() -> None:
    vm = machine()
    heap = 0x300000
    vm.mem_map(heap, 0x200000)

    def put(address: int, value: int, width: int = 8) -> None:
        vm.mem_write(address, value.to_bytes(width, "little"))

    def get(address: int, width: int = 8) -> int:
        return int.from_bytes(vm.mem_read(address, width), "little")

    def node(next_ptr: int, prev_ptr: int, value: int) -> int:
        nonlocal heap
        addr = heap
        heap += 40
        vm.mem_write(addr, struct.pack("<QQQ", next_ptr, prev_ptr, value))
        return addr

    for offset in (0xb00, 0xaf0, 0xcb0):
        anchor = node(heap, heap, 0)
        put(STATE + offset, anchor)
    put(STATE + 0xccc, 1, 4)
    put(STATE + 0xcc8, 0x42b40000, 4)
    put(STATE + 0xcd0, 50, 4)
    put(STATE + 0xcd4, 5, 4)
    put(STATE + 0xcd8, 50, 4)
    put(STATE + 0xce0, 0x3f800000, 4)
    put(STATE + 0xce4, 0x3f800000, 4)
    pending_head = get(STATE + 0xaf0)
    def helper(uc, addr: int, size: int, _: object) -> None:
        pc = addr - BASE
        obj = uc.reg_read(reg.UC_X86_REG_RCX)
        rax = 0
        if pc in (0x226a4, 0x22710):
            rax = time_ns // 100
        elif pc == 0x22792:  # splice right-eye ATW records
            anchor = get(obj)
            for _ in range(records):
                last = get(anchor + 8)
                new = node(anchor, last, 0)
                put(last, new)
                put(anchor + 8, new)
            put(obj + 8, get(obj + 8) + records)
        elif pc in (0x227b3, 0x22829, 0x22ba1, 0x22c9d):
            anchor = get(obj)
            put(anchor, anchor)
            put(anchor + 8, anchor)
            put(obj + 8, 0)
        elif pc in (0x228ae, 0x22b49):
            rax = node(uc.reg_read(reg.UC_X86_REG_RDX), uc.reg_read(reg.UC_X86_REG_R8),
                       get(uc.reg_read(reg.UC_X86_REG_R9)))
        uc.reg_write(reg.UC_X86_REG_RAX, rax)
        uc.reg_write(reg.UC_X86_REG_RIP, addr + size)
    for pc in (0x22692, 0x226a4, 0x226fd, 0x22710, 0x22792, 0x227b3, 0x22829,
               0x2285e, 0x228ae, 0x2293a, 0x2298d, 0x22b49, 0x22ba1, 0x22bad, 0x22c9d):
        vm.hook_add(UC_HOOK_CODE, helper, begin=BASE+pc, end=BASE+pc)
    rows = []
    duplicates = 0
    put(FRAME + 0x328, FRAME + 0x800)
    for i in range(2201):
        time_ns = 1_000_000_000 + 11_111_100 * i
        vc = 1 + i - int(i >= 500)
        records = 1 + int(i % 19 == 0) * 2
        pll = int(5 <= i < 1300 or i > 1320)
        duplicates += int((i // 100) % 3 == 1 and i % 2 == 0)
        put(STATE + 0x1588, pll, 1)
        put(FRAME + 0x330, records)
        put(FRAME + 0x358, vc)
        put(FRAME + 0x360, duplicates)
        vm.reg_write(reg.UC_X86_REG_RSI, STATE)
        vm.reg_write(reg.UC_X86_REG_RBP, FRAME)
        vm.reg_write(reg.UC_X86_REG_RSP, STATE + 0x7800)
        vm.reg_write(reg.UC_X86_REG_R12, 0)
        vm.emu_start(BASE + 0x22613, BASE + 0x22ca2, count=20000)
        assert vm.reg_read(reg.UC_X86_REG_RIP) == BASE + 0x22ca2
        raw = get(get(pending_head+8)+16, 4) if get(STATE+0xaf8) else -1
        period = int(struct.unpack("<f", struct.pack("<f", 1e9 /
                     struct.unpack("<f", vm.mem_read(STATE+0xcc8,4))[0]))[0])
        rows.append((time_ns,vc,duplicates,records,pll,get(STATE+0xb08),get(STATE+0xcdc,1),
                     get(STATE+0xca8),get(STATE+0xcc0),get(STATE+0xce0,4),
                     get(STATE+0xce4,4),raw,period))
        put(pending_head, pending_head)
        put(pending_head+8,pending_head)
        put(STATE+0xaf8,0)
    Path(__file__).with_name("trajectory.tsv").write_text(
        "".join("\t".join(map(str,row))+"\n" for row in rows))
    print(len(rows), "state transitions")


if __name__ == "__main__":
    main()
