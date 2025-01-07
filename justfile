cyw43-dev:
	probe-rs download cyw43-firmware/43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
	probe-rs download cyw43-firmware/43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
	probe-rs download cyw43-firmware/43439A0_btfw.bin --binary-format bin --chip RP2040 --base-address 0x10141400
