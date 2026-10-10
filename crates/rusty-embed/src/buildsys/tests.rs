use super::*;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Enough of an ELF header for `embedded_elf`: the magic, little-endian,
/// the file type and the machine.
fn elf(root: &Path, path: &str, kind: u16, machine: u16) {
    let mut head = vec![0u8; 64];
    head[..4].copy_from_slice(b"\x7fELF");
    head[4] = 1;
    head[5] = 1;
    head[16..18].copy_from_slice(&kind.to_le_bytes());
    head[18..20].copy_from_slice(&machine.to_le_bytes());
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, head).unwrap();
}

fn args(plan: &CommandPlan) -> String {
    plan.args.join(" ")
}

/// Cargo's file wins, then PlatformIO's, then CMake's — and a directory with
/// none of them is no project.
#[test]
fn the_file_at_the_root_decides_the_build_system() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(system_at(dir.path()), None);
    write(dir.path(), "CMakeLists.txt", "project(x)\n");
    assert_eq!(system_at(dir.path()), Some(BuildSystem::Cmake));
    write(dir.path(), "platformio.ini", "[env:a]\n");
    assert_eq!(system_at(dir.path()), Some(BuildSystem::PlatformIo));
    write(dir.path(), "Cargo.toml", "[package]\nname = \"x\"\n");
    assert_eq!(system_at(dir.path()), Some(BuildSystem::Cargo));

    let none = tempfile::tempdir().unwrap();
    let refused = crate::project::detect(none.path()).unwrap_err().to_string();
    for file in ["Cargo.toml", "platformio.ini", "CMakeLists.txt"] {
        assert!(refused.contains(file), "{refused}");
    }
}

/// The environment built is `default_envs`' first; its keys are `[env]`'s,
/// then what it extends, then its own; and the board names the chip through
/// the catalogue, with no PlatformIO installed.
#[test]
fn a_platformio_environment_is_read_as_platformio_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "platformio.ini",
        "; a comment\n\
         [platformio]\n\
         default_envs = c3, other\n\
         \n\
         [env]\n\
         framework = arduino\n\
         monitor_speed = 115200 ; inline comment\n\
         \n\
         [base]\n\
         platform = espressif32\n\
         \n\
         [env:other]\n\
         board = esp32dev\n\
         \n\
         [env:c3]\n\
         extends = base\n\
         board = esp32-c3-devkitm-1\n\
         build_flags =\n\
         \x20   -DONE\n\
         \x20   -DTWO\n",
    );
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(project.build.system, BuildSystem::PlatformIo);
    assert_eq!(project.build.environment.as_deref(), Some("c3"));
    assert_eq!(project.build.environments, ["other", "c3"]);
    assert_eq!(project.chip.as_deref(), Some("esp32c3"));
    assert!(
        project
            .chip_source
            .as_deref()
            .unwrap()
            .contains("esp32-c3-devkitm-1"),
        "{:?}",
        project.chip_source
    );
    assert!(
        project
            .frameworks
            .iter()
            .any(|f| f.starts_with("espressif32"))
    );
    assert!(project.frameworks.iter().any(|f| f.starts_with("arduino")));
    assert!(project.problems.is_empty(), "{:?}", project.problems);

    let sections = parse_ini(&std::fs::read_to_string(dir.path().join("platformio.ini")).unwrap());
    let c3 = environment(&sections, "c3");
    assert_eq!(c3["build_flags"], "-DONE\n-DTWO", "a continued value");
    assert_eq!(
        c3["monitor_speed"], "115200",
        "an inline comment is not the value"
    );
    assert_eq!(c3["platform"], "espressif32", "extends");

    let plans = build_plans(&project, dir.path()).unwrap();
    assert_eq!(plans.len(), 2, "a build, then the compile database once");
    assert_eq!(plans[0].program, "pio");
    assert_eq!(args(&plans[0]), "run -e c3");
    assert_eq!(args(&plans[1]), "run -e c3 -t compiledb");
    write(dir.path(), "compile_commands.json", "[]");
    assert_eq!(build_plans(&project, dir.path()).unwrap().len(), 1);
    assert_eq!(
        compile_commands_dir(dir.path(), &project).as_deref(),
        Some(dir.path())
    );
}

/// clangd may run the cross compilers the catalogue names, and no other.
#[test]
fn clangd_may_run_the_catalogues_cross_compilers_only() {
    let globs = query_driver_globs(&Catalog::builtin());
    for glob in [
        "**/arm-none-eabi-*",
        "**/riscv32-esp-elf-*",
        "**/xtensa-esp-elf-*",
        "**/xtensa-esp*-elf-*",
    ] {
        assert!(globs.iter().any(|g| g == glob), "{glob}: {globs:?}");
    }
    assert!(globs.iter().all(|g| g != "**/*"), "{globs:?}");
}

/// `board_build.mcu` names the part outright, by its ordering code; a board
/// nothing describes is a warning that says what would name it, not a guess.
#[test]
fn a_platformio_chip_is_named_or_said_to_be_unknown() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "platformio.ini",
        "[env:pill]\nplatform = ststm32\nboard = genericSTM32F411CE\nboard_build.mcu = stm32f411ceu6\n",
    );
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(project.chip.as_deref(), Some("stm32f411ce"));

    let unknown = tempfile::tempdir().unwrap();
    write(
        unknown.path(),
        "platformio.ini",
        "[env:mystery]\nboard = some_board_nobody_has_12345\n",
    );
    let project = crate::project::detect(unknown.path()).unwrap();
    assert_eq!(project.chip, None);
    assert_eq!(project.problems[0].kind, "pio-chip-unknown");
    assert_eq!(
        project.problems[0].args["board"],
        "some_board_nobody_has_12345"
    );

    let empty = tempfile::tempdir().unwrap();
    write(empty.path(), "platformio.ini", "[platformio]\n");
    let project = crate::project::detect(empty.path()).unwrap();
    assert_eq!(project.problems[0].kind, "pio-env-none");
    assert!(build_plans(&project, empty.path()).is_err());
}

/// PlatformIO's own upload, handed the port the title bar chose and
/// nothing about a probe, which the environment's `upload_protocol` picks.
#[test]
fn a_platformio_project_is_flashed_by_platformio() {
    let port = Transport::Serial {
        port: "COM7".into(),
    };
    let probe = Transport::Probe { identifier: None };
    let flash = platformio_flash("c3", &port, FlashAction::FlashAndMonitor, None);
    assert_eq!(flash.program, "pio");
    assert_eq!(
        args(&flash),
        "run -e c3 -t upload -t monitor --upload-port COM7 --monitor-port COM7"
    );
    assert_eq!(
        args(&platformio_flash("c3", &probe, FlashAction::Flash, None)),
        "run -e c3 -t upload"
    );
    assert_eq!(
        args(&platformio_flash(
            "c3",
            &port,
            FlashAction::Monitor,
            Some(115_200)
        )),
        "device monitor -e c3 -p COM7 -b 115200"
    );
}

/// The Pico SDK's default board is the Pico; PICO_BOARD and PICO_PLATFORM
/// move it. The build configures once, with Ninja and compile commands,
/// and only builds after that.
#[test]
fn a_pico_sdk_project_names_its_chip_and_configures_once() {
    let dir = tempfile::tempdir().unwrap();
    let lists = "cmake_minimum_required(VERSION 3.13)\n\
                 include(pico_sdk_import.cmake)\n\
                 project(blink C CXX ASM)\n\
                 pico_sdk_init()\n";
    write(dir.path(), "CMakeLists.txt", lists);
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(project.build.system, BuildSystem::Cmake);
    assert_eq!(project.build.sdk, Some(CmakeSdk::PicoSdk));
    assert_eq!(project.chip.as_deref(), Some("rp2040"));

    let plans = build_plans(&project, dir.path()).unwrap();
    assert_eq!(plans.len(), 2, "configure, then build");
    assert_eq!(
        args(&plans[0]),
        "-S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_EXPORT_COMPILE_COMMANDS=ON"
    );
    assert_eq!(args(&plans[1]), "--build build");
    write(dir.path(), "build/CMakeCache.txt", "");
    assert_eq!(build_plans(&project, dir.path()).unwrap().len(), 1);

    write(
        dir.path(),
        "CMakeLists.txt",
        &format!("set(PICO_BOARD pico2)\n{lists}"),
    );
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(project.chip.as_deref(), Some("rp235xa"));

    write(
        dir.path(),
        "CMakeLists.txt",
        &format!("set(PICO_BOARD my_own_board)\n{lists}"),
    );
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(
        project.chip, None,
        "a board the list does not know is not guessed"
    );
    assert_eq!(project.problems[0].kind, "cmake-chip-unknown");
}

/// ESP-IDF's target is `sdkconfig`'s, and `idf.py` builds it.
#[test]
fn an_esp_idf_project_reads_its_target_from_sdkconfig() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "CMakeLists.txt",
        "cmake_minimum_required(VERSION 3.16)\n\
         include($ENV{IDF_PATH}/tools/cmake/project.cmake)\n\
         project(hello)\n",
    );
    write(
        dir.path(),
        "sdkconfig",
        "# comment\nCONFIG_IDF_TARGET=\"esp32c3\"\n",
    );
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(project.build.sdk, Some(CmakeSdk::EspIdf));
    assert_eq!(project.chip.as_deref(), Some("esp32c3"));
    let plans = build_plans(&project, dir.path()).unwrap();
    assert_eq!(plans[0].program, "idf.py");
    assert_eq!(args(&plans[0]), "build");
}

/// STM32CubeMX's `.ioc` names the part by its ordering code, and its
/// presets say where the build goes — through `inherits`.
#[test]
fn a_cube_project_reads_its_part_and_its_preset() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "CMakeLists.txt",
        "cmake_minimum_required(VERSION 3.22)\nproject(app)\n",
    );
    write(
        dir.path(),
        "app.ioc",
        "Mcu.Family=STM32F4\nMcu.CPN=STM32F411CEU6\nMcu.UserName=STM32F411CEUx\n",
    );
    write(
        dir.path(),
        "CMakePresets.json",
        r#"{
          "version": 3,
          "configurePresets": [
            { "name": "default", "hidden": true, "generator": "Ninja",
              "binaryDir": "${sourceDir}/build/${presetName}" },
            { "name": "Debug", "inherits": "default" },
            { "name": "Release", "inherits": "default" }
          ]
        }"#,
    );
    let project = crate::project::detect(dir.path()).unwrap();
    assert_eq!(project.build.sdk, Some(CmakeSdk::Stm32Cube));
    assert_eq!(project.chip.as_deref(), Some("stm32f411ce"));
    assert_eq!(project.build.preset.as_deref(), Some("Debug"));
    assert_eq!(project.build.build_dir.as_deref(), Some("build/Debug"));
    let plans = build_plans(&project, dir.path()).unwrap();
    assert_eq!(
        args(&plans[0]),
        "--preset Debug -DCMAKE_EXPORT_COMPILE_COMMANDS=ON"
    );
    assert_eq!(args(&plans[1]), "--build build/Debug");
}

/// A build directory's firmware is the Arm, RISC-V or Xtensa executable in
/// it — not its object files, not a host tool, not ESP-IDF's bootloader.
#[test]
fn only_the_app_is_taken_from_a_build_directory() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "CMakeLists.txt", "pico_sdk_init()\n");
    elf(dir.path(), "build/blink.elf", 2, 40);
    elf(dir.path(), "build/CMakeFiles/blink.dir/main.c.obj", 1, 40);
    elf(dir.path(), "build/main.o", 1, 40);
    elf(dir.path(), "build/pioasm/pioasm", 2, 62);
    elf(dir.path(), "build/bootloader/bootloader.elf", 2, 243);
    let project = crate::project::detect(dir.path()).unwrap();
    let found = images(dir.path(), &project);
    let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["blink"]);

    let pio = tempfile::tempdir().unwrap();
    write(
        pio.path(),
        "platformio.ini",
        "[env:c3]\nboard = esp32-c3-devkitm-1\n",
    );
    elf(pio.path(), ".pio/build/c3/firmware.elf", 2, 243);
    let project = crate::project::detect(pio.path()).unwrap();
    let found = images(pio.path(), &project);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].target, "c3");
    assert!(found[0].matches_configured_target);
}

/// The C in a Cargo project — what a `cc` build script compiles — is found
/// past the build directory and the dot entries, and compiled in the
/// database by the part's cross compiler: `g++` for C++, freestanding, the
/// header directories on the include path, and no entry for a header.
#[test]
fn a_cargo_projects_c_is_given_a_compile_database_by_its_parts_compiler() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "Cargo.toml", "[package]\nname = \"mixed\"\n");
    write(root, "src/main.rs", "fn main() {}\n");
    write(root, "csrc/vendor.c", "int x;\n");
    write(root, "csrc/vendor.h", "int f(void);\n");
    write(root, "csrc/filter.cpp", "int y;\n");
    write(root, "target/debug/build/cc/out.c", "int stale;\n");
    write(root, ".git/hooks/sample.c", "int hook;\n");

    let sources = c_sources(root);
    assert_eq!(
        sources,
        ["csrc/filter.cpp", "csrc/vendor.c", "csrc/vendor.h"]
    );

    let database = cargo_compile_commands(root, &sources, Some("arm-none-eabi-gcc"));
    let entries = database.as_array().unwrap();
    assert_eq!(entries.len(), 2, "a header is not compiled: {database:#}");
    let arguments = |file: &str| -> Vec<String> {
        let entry = entries
            .iter()
            .find(|e| e["file"].as_str().unwrap().ends_with(file))
            .unwrap();
        serde_json::from_value(entry["arguments"].clone()).unwrap()
    };
    let c = arguments("vendor.c");
    assert_eq!(c[0], "arm-none-eabi-gcc");
    assert!(c.contains(&"-ffreestanding".to_string()), "{c:?}");
    let include = format!("-I{}", root.join("csrc").display());
    assert!(c.contains(&include), "{c:?}");
    let cpp = arguments("filter.cpp");
    assert_eq!(cpp[0], "arm-none-eabi-g++");
    assert!(cpp.contains(&"-fno-exceptions".to_string()), "{cpp:?}");

    // A host crate with C has no part, and the host's compiler is named.
    let host = cargo_compile_commands(root, &sources, None);
    assert_eq!(host[0]["arguments"][0], "c++");
    assert!(
        !host[0]["arguments"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "-ffreestanding")
    );
}

/// A Cargo project with no C starts no clangd: nothing is written.
#[test]
fn a_cargo_project_without_c_gets_no_compile_database() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Cargo.toml", "[package]\nname = \"plain\"\n");
    write(dir.path(), "src/main.rs", "fn main() {}\n");
    write(dir.path(), "include/only.h", "int f(void);\n");
    assert_eq!(write_cargo_compile_commands(dir.path(), None), None);
}

/// A part's toolchain file names its cross compiler and its core's flags,
/// from the catalogue — and a part with no flags on file gets none.
#[test]
fn a_toolchain_file_carries_the_parts_compiler_and_core() {
    let f411 = crate::chip::by_id("stm32f411ce").unwrap();
    let text = toolchain_file(&f411).unwrap();
    for line in [
        "set(CMAKE_SYSTEM_NAME Generic)",
        "set(CMAKE_SYSTEM_PROCESSOR arm)",
        "set(CMAKE_C_COMPILER arm-none-eabi-gcc)",
        "set(CMAKE_CXX_COMPILER arm-none-eabi-g++)",
        "set(CMAKE_OBJCOPY arm-none-eabi-objcopy)",
        "set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)",
        "set(CORE_FLAGS \"-mcpu=cortex-m4 -mthumb -mfloat-abi=hard -mfpu=fpv4-sp-d16\")",
    ] {
        assert!(text.lines().any(|l| l == line), "{line} in\n{text}");
    }
    let m0 = toolchain_file(&crate::chip::by_id("rp2040").unwrap()).unwrap();
    assert!(m0.contains("-mcpu=cortex-m0plus"), "{m0}");
    assert!(!m0.contains("-mfpu"), "a Cortex-M0+ has no FPU: {m0}");
    assert_eq!(toolchain_file(&crate::chip::by_id("esp32").unwrap()), None);
}

/// A plain CMake project names its part with `set(RUSTY_CHIP …)`; with a
/// part rusty has flags for, the toolchain file is written once — never
/// over one that is there — and the configure step names it.
#[test]
fn a_cmake_project_that_names_no_compiler_is_given_the_parts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "CMakeLists.txt",
        "cmake_minimum_required(VERSION 3.20)\n\
         set(RUSTY_CHIP stm32f411ce)\n\
         project(blink C ASM)\n\
         add_executable(blink main.c)\n",
    );
    let project = crate::project::detect(root).unwrap();
    assert_eq!(project.chip.as_deref(), Some("stm32f411ce"));
    assert!(
        project
            .chip_source
            .as_deref()
            .is_some_and(|s| s.contains("RUSTY_CHIP")),
        "{:?}",
        project.chip_source
    );
    assert!(!names_its_toolchain(root, &project.build));

    let plans = build_plans(&project, root).unwrap();
    assert!(
        args(&plans[0]).ends_with("-DCMAKE_TOOLCHAIN_FILE=cmake/rusty-toolchain.cmake"),
        "{}",
        args(&plans[0])
    );

    assert_eq!(
        ensure_toolchain_file(&project, root).unwrap().as_deref(),
        Some(TOOLCHAIN_FILE)
    );
    let written = std::fs::read_to_string(root.join(TOOLCHAIN_FILE)).unwrap();
    assert!(written.contains("arm-none-eabi-gcc"), "{written}");

    // The project's now: an edit stays, and nothing is written again.
    write(root, TOOLCHAIN_FILE, "# mine\n");
    assert_eq!(ensure_toolchain_file(&project, root).unwrap(), None);
    assert_eq!(
        std::fs::read_to_string(root.join(TOOLCHAIN_FILE)).unwrap(),
        "# mine\n"
    );
}

/// A project that names its own compiler — in CMakeLists.txt or a preset —
/// is left alone, and one for a part rusty has no flags for is refused
/// rather than configured for this machine.
#[test]
fn a_named_toolchain_is_left_alone_and_a_missing_one_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "CMakeLists.txt",
        "set(CMAKE_TOOLCHAIN_FILE ${CMAKE_SOURCE_DIR}/arm.cmake)\n\
         set(RUSTY_CHIP stm32f411ce)\n\
         project(blink C)\n",
    );
    let project = crate::project::detect(root).unwrap();
    assert!(names_its_toolchain(root, &project.build));
    assert_eq!(ensure_toolchain_file(&project, root).unwrap(), None);
    assert!(!root.join(TOOLCHAIN_FILE).exists());
    let plans = build_plans(&project, root).unwrap();
    assert!(!args(&plans[0]).contains("CMAKE_TOOLCHAIN_FILE"));

    let other = tempfile::tempdir().unwrap();
    write(
        other.path(),
        "CMakeLists.txt",
        "set(RUSTY_CHIP stm32f411ce)\nproject(blink C)\n",
    );
    write(
        other.path(),
        "CMakePresets.json",
        r#"{"version": 3, "configurePresets": [{"name": "arm", "toolchainFile": "arm.cmake"}]}"#,
    );
    let project = crate::project::detect(other.path()).unwrap();
    assert!(names_its_toolchain(other.path(), &project.build));

    let esp = tempfile::tempdir().unwrap();
    write(
        esp.path(),
        "CMakeLists.txt",
        "set(RUSTY_CHIP esp32)\nproject(blink C)\n",
    );
    let project = crate::project::detect(esp.path()).unwrap();
    assert_eq!(project.chip.as_deref(), Some("esp32"));
    assert_eq!(ensure_toolchain_file(&project, esp.path()).unwrap(), None);
    let refused = build_plans(&project, esp.path()).unwrap_err().to_string();
    assert!(refused.contains("names no compiler"), "{refused}");
}
