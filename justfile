cyw43-firmware := "https://github.com/embassy-rs/embassy/blob/main/cyw43-firmware/"
cyw43-base := cyw43-firmware + "43439A0.bin" 
cyw43-clm := cyw43-firmware + "43439A0_clm.bin"
cyw43-btfw := cyw43-firmware + "43439A0_btfw.bin"

cyw43-dev:
	mkdir -p cyw43-firmware/
	wget {{cyw43-base}} -O cyw43-firmware/43439A0.bin 
	wget {{cyw43-clm}} -O cyw43-firmware/43439A0_clm.bin
	wget {{cyw43-btfw}} -O cyw43-firmware/43439A0_btfw.bin

	probe-rs download cyw43-firmware/43439A0.bin --binary-format bin --chip RP2040 --base-address 0x10100000
	probe-rs download cyw43-firmware/43439A0_clm.bin --binary-format bin --chip RP2040 --base-address 0x10140000
	probe-rs download cyw43-firmware/43439A0_btfw.bin --binary-format bin --chip RP2040 --base-address 0x10141400
