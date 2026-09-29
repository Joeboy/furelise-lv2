PROFILE ?= release
TARGET_DIR ?= target
BUILD_DIR ?= build
STRIP ?= 1
BUNDLE := $(BUILD_DIR)/furelise.lv2
PICOLV2_SDK_DIR ?= ../../sdk
PICO_BUNDLE_ROOT ?= $(BUILD_DIR)/picolv2
PICO_BUNDLE := $(PICO_BUNDLE_ROOT)/furelise.lv2
PICO_BUILD_DIR := $(BUILD_DIR)/pico-strip$(STRIP)
PICO_OBJECT := $(PICO_BUILD_DIR)/plugin.o
PICO_LIBRARY := $(PICO_BUILD_DIR)/plugin.so
PICO_STRIP_FLAG := $(if $(filter 0,$(STRIP)),,-Wl,--strip-debug)

ifeq ($(OS),Windows_NT)
LIBRARY := furelise_lv2.dll
else ifeq ($(shell uname -s),Darwin)
LIBRARY := libfurelise_lv2.dylib
else
LIBRARY := libfurelise_lv2.so
endif

.PHONY: bundle bundle-pico clean

bundle:
	cargo build --profile $(PROFILE)
	mkdir -p $(BUNDLE)
	sed 's/@BINARY@/$(LIBRARY)/' furelise.lv2/manifest.ttl > $(BUNDLE)/manifest.ttl
	cp furelise.lv2/furelise.ttl $(BUNDLE)/furelise.ttl
	cp furelise.lv2/FurElise.MID $(BUNDLE)/FurElise.MID
	cp $(TARGET_DIR)/$(PROFILE)/$(LIBRARY) $(BUNDLE)/$(LIBRARY)

$(PICO_BUILD_DIR):
	mkdir -p $@

$(PICO_OBJECT): src/picolv2.rs furelise.lv2/FurElise.MID | $(PICO_BUILD_DIR)
	rustc --edition=2021 --target thumbv8m.main-none-eabihf --crate-type lib --emit=obj \
		-C panic=abort -C opt-level=2 -C overflow-checks=no -C relocation-model=pic \
		-o $@ $<

$(PICO_LIBRARY): $(PICO_OBJECT) $(PICOLV2_SDK_DIR)/src/runtime.c
	arm-none-eabi-gcc -mcpu=cortex-m33 -mthumb -mfloat-abi=hard -mfpu=fpv5-sp-d16 \
		-shared -nostdlib -Wl,-Bsymbolic -Wl,-z,undefs \
		-Wl,-z,max-page-size=0x1000 -Wl,--no-warnings $(PICO_STRIP_FLAG) \
		-o $@ $^ -Wl,--start-group -lc -lm -lnosys -lgcc -Wl,--end-group

bundle-pico: $(PICO_LIBRARY)
	mkdir -p $(PICO_BUNDLE)
	sed 's/@BINARY@/plugin.so/' furelise.lv2/manifest.ttl > $(PICO_BUNDLE)/manifest.ttl
	cp furelise.lv2/furelise.ttl furelise.lv2/FurElise.MID $(PICO_BUNDLE)/
	cp $(PICO_LIBRARY) $(PICO_BUNDLE)/plugin.so

clean:
	cargo clean
	rm -rf $(BUILD_DIR)
