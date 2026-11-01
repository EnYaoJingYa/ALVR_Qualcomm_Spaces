"""Replay the installed RVRPlugin.dll arithmetic blocks, without loading the DLL."""

import hashlib
import struct
from pathlib import Path

from unicorn import Uc, UC_ARCH_X86, UC_MODE_64, UC_HOOK_CODE
from unicorn import x86_const as reg

DLL = Path(r"C:\Program Files (x86)\ThinkRealityStreamingAssitant\driver\svr\bin\win64\RVRPlugin.dll")
SHA = "8b59e2ca72f8241595d6ce1075193d78e471f5475c9bbf8804656ec5ffef3a27"
BASE = 0x180000000
STATE = 0x100000
FRAME = STATE + 0x4000


def machine() -> Uc:
    data = DLL.read_bytes()
    if hashlib.sha256(data).hexdigest() != SHA:
        raise ValueError("Unrecognized DLL")
    pe = struct.unpack_from("<I", data, 60)[0]
    section_count = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    vm = Uc(UC_ARCH_X86, UC_MODE_64)
    vm.mem_map(BASE, 0x100000)
    for i in range(section_count):
        offset = pe + 24 + optional_size + 40 * i
        va, length, start = struct.unpack_from("<III", data, offset + 12)
        if length:
            vm.mem_write(BASE + va, data[start:start + length])
    vm.mem_map(STATE, 0x8000)
    return vm


def replay(vm: Uc, previous: int, current: int, last_cost: int,
           minimum: int, direction: int) -> tuple[int, int, int]:
    vm.reg_write(reg.UC_X86_REG_RSI, STATE)
    vm.reg_write(reg.UC_X86_REG_RBP, FRAME)
    vm.reg_write(reg.UC_X86_REG_R15, current)
    vm.mem_write(STATE + 0xCA0, struct.pack("<QQ", previous, last_cost))
    vm.mem_write(STATE + 0xCC0, struct.pack("<Q", minimum))
    vm.mem_write(STATE + 0xCD8, struct.pack("<I", 50))
    vm.mem_write(STATE + 0xCE0, struct.pack("<f", direction))
    vm.mem_write(FRAME + 0x198, bytes(8))
    for register, value in [(reg.UC_X86_REG_XMM11, 0.5),
                            (reg.UC_X86_REG_XMM9, float(2**64))]:
        vm.reg_write(register, int.from_bytes(struct.pack("<f", value), "little"))
    vm.emu_start(BASE + 0x22876, BASE + 0x22896)
    cost = int.from_bytes(vm.mem_read(FRAME + 0x198, 8), "little")
    vm.emu_start(BASE + 0x2299F, BASE + 0x22B31)
    bits = int.from_bytes(vm.mem_read(FRAME + 0x190, 4), "little")
    vm.reg_write(reg.UC_X86_REG_XMM6, bits)
    vm.mem_write(FRAME + 0xC50, struct.pack("<Q", 10_500_000))
    vm.emu_start(BASE + 0x32D0C, BASE + 0x32D1F)
    target = int.from_bytes(vm.mem_read(FRAME + 0xC50, 8), "little")
    return cost, bits, target - 10_500_000


def replay_wait(previous: int, now: int, raw: float, left: bool, pll: bool,
                advance: int = 0) -> tuple[int, int]:
    """Execute the whole waiter with virtual time, heap nodes and mutex stubs."""
    vm = machine()
    ctl, root, node, head = STATE + 0x2000, FRAME, FRAME + 0x40, FRAME + 0x80
    def put(addr: int, value: int) -> None:
        vm.mem_write(addr, struct.pack("<Q", value))
    def get(addr: int) -> int:
        return int.from_bytes(vm.mem_read(addr, 8), "little")
    put(STATE + 0xbb0, ctl)
    put(STATE + 0xc50, previous)
    put(STATE + 0x968, 1)
    put(STATE + 0x9e0, advance)
    put(STATE + 0x10c8, int(pll))
    put(STATE + 0x10b8, head)
    put(head, head)
    put(head + 8, head)
    put(ctl + 0xaf0, root)
    put(ctl + 0xaf8, 1)
    put(root, node)
    put(root + 8, node)
    put(node, root)
    put(node + 8, root)
    vm.mem_write(node + 16, struct.pack("<f", raw))
    stack = STATE + 0x7e08
    put(stack, BASE + 0x90000)
    vm.reg_write(reg.UC_X86_REG_RSP, stack)
    vm.reg_write(reg.UC_X86_REG_RCX, STATE)
    vm.reg_write(reg.UC_X86_REG_RDX, int(left))
    reached = []
    def external(uc: Uc, addr: int, size: int, _: object) -> None:
        nonlocal now
        pc = addr - BASE
        if pc == 0x32a26:
            uc.reg_write(reg.UC_X86_REG_XMM0, int.from_bytes(struct.pack("<f", 90), "little"))
        elif pc == 0x32ab8:
            allocated = head + 64
            put(allocated, head)
            put(allocated + 8, head)
            put(allocated + 16, get(uc.reg_read(reg.UC_X86_REG_R9)))
            uc.reg_write(reg.UC_X86_REG_RAX, allocated)
        elif pc in {0x32c7c, 0x32cf0, 0x32cd8}:
            uc.reg_write(reg.UC_X86_REG_RAX, 0)
        else:
            if pc == 0x32dc1:
                target = get(STATE + 0xc50) + uc.reg_read(reg.UC_X86_REG_R13)
                reached.append(target)
                now = max(now, ((target + 99) // 100) * 100)
            uc.reg_write(reg.UC_X86_REG_RAX, now // 100)
        uc.reg_write(reg.UC_X86_REG_RIP, addr + size)
    for pc in (0x32a26, 0x32a3c, 0x32a57, 0x32a8c, 0x32ab8, 0x32bf9, 0x32c13,
               0x32c42, 0x32c7c, 0x32cd8, 0x32cf0, 0x32dc1):
        vm.hook_add(UC_HOOK_CODE, external, begin=BASE + pc, end=BASE + pc)
    vm.emu_start(BASE + 0x329e0, BASE + 0x90000, count=10000)
    assert vm.reg_read(reg.UC_X86_REG_RIP) == BASE + 0x90000
    return reached[-1], get(STATE + 0xc50)


def main() -> None:
    vm = machine()
    rows = []
    for previous in [0, 100]:
        for delta in range(56):
            for last in [0, delta, 56]:
                for minimum in sorted({0, min(delta, 5), min(delta, 50), delta, max(0,delta-1)}):
                    for direction in [-1, 1]:
                        # Zero sentinel forces cost=0; history min cannot exceed cost.
                        if not previous and minimum:
                            continue
                        args = previous, previous + delta, last, minimum, direction
                        rows.append((*args, *replay(vm, *args)))
    path = Path(__file__).with_name("oracle.tsv")
    path.write_text("".join("\t".join(map(str, row)) + "\n" for row in rows))
    print(len(rows), "vectors; signed adjustments (ns)", sorted({r[-1] for r in rows}))
    print("zero sentinel:", replay(vm, 0, 11, 0, 0, 1))
    waits = []
    for left in (False, True):
        for pll in (False, True):
            for previous in (0, 9_500_000, 40_000_000):
                for raw in (-2., -1., 0., 1.):
                    for advance in (0, 300_000):
                        target, saved = replay_wait(previous, 20_000_000, raw, left, pll, advance)
                        waits.append((previous, 20_000_000, int(raw), int(left), int(pll), advance, target, saved))
    Path(__file__).with_name("wait.tsv").write_text("".join("\t".join(map(str,r))+"\n" for r in waits))
    print(len(waits), "waiter vectors (positive input injected at consumer only)")


if __name__ == "__main__":
    main()
