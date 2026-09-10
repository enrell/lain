cmake_minimum_required(VERSION 3.21)
project(lain_client VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

find_package(Qt6 6.5 REQUIRED COMPONENTS Quick Qml OpenGL)
find_package(PkgConfig REQUIRED)
pkg_check_modules(MPV REQUIRED IMPORTED_TARGET mpv)

qt_standard_project_setup(REQUIRES 6.5)
qt_policy(SET QTP0004 NEW)
string(TIMESTAMP LAIN_BUILD_TS "%Y-%m-%d %H:%M:%S")

set_source_files_properties(qml/theme/Tokens.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)

qt_add_executable(lain
    src/main.cpp
    src/MpvItem.h src/MpvItem.cpp
    src/ServerClient.h src/ServerClient.cpp
    src/OmarchyTheme.h src/OmarchyTheme.cpp
)

qt_add_qml_module(lain
    URI Lain
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/theme/Tokens.qml
        qml/theme/Color.js
        qml/components/PageHeader.qml
        qml/components/NavRail.qml
        qml/components/MediaRow.qml
        qml/components/PosterCard.qml
        qml/components/LandscapeCard.qml
        qml/components/PersonCard.qml
        qml/components/PlayerButton.qml
        qml/components/SeekBar.qml
        qml/components/Hero.qml
        qml/views/HomeView.qml
        qml/views/LibraryView.qml
        qml/views/DetailView.qml
        qml/views/PlayerView.qml
        qml/views/SettingsView.qml
        qml/views/SearchOverlay.qml
)

target_link_libraries(lain PRIVATE
    Qt6::Quick Qt6::Qml Qt6::OpenGL
    PkgConfig::MPV
)
target_include_directories(lain PRIVATE src)
target_compile_definitions(lain PRIVATE LAIN_BUILD_TS="${LAIN_BUILD_TS}")

install(TARGETS lain RUNTIME DESTINATION bin)
