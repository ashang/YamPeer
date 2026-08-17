# Requirements Document

## Introduction

Image Editor 是一款面向 macOS 和 Linux 用户的桌面图片浏览与基础编辑应用。用户可以从文件夹浏览图片，以键盘为主要操作方式完成翻转、旋转、裁剪及明暗调整，并将编辑结果导出为新文件。应用应在 macOS 和 Linux 上提供由检测到的图片格式和平台集成能力决定的浏览、编辑和导出能力；不可用能力应以可见状态说明和禁用的相关操作表示，而不阻止应用启动。

用户指定的实现约束：后续设计与实现应采用 Rust 共享图片处理和应用行为，并提供高效、简洁的现代桌面前端。本需求阶段保留该约束，但不预设具体框架、图像处理库、系统 API、图像编解码器或界面实现方案。

## Glossary

- **Image_Editor**：本功能提供的 macOS 和 Linux 图片浏览和基础编辑桌面应用。
- **Supported_Platform**：安装并运行 Image_Editor 的 macOS 或 Linux 操作系统实例；本术语不假定特定 Linux 桌面环境、窗口系统、系统 API 或系统图像编解码器可用。
- **Platform_Integration_Capability**：Image_Editor 在当前 Supported_Platform 上检测到的、支持特定本地交互的能力，例如选择本地文件夹或选择本地导出文件路径。
- **Platform_File_Chooser**：当前 Platform_Integration_Capability 支持时，Supported_Platform 为选择一个本地文件夹或一个本地导出文件路径提供的文件选择界面。
- **Unavailable_Platform_Capability**：当前 Supported_Platform 上未检测到的 Platform_Integration_Capability。
- **Dependent_Operation**：依赖特定 Image_Format_Capability 或 Platform_Integration_Capability 的 open-folder、图像选择、图像编辑或 export 操作。
- **Platform_Keyboard_Command**：用户在 Image_Editor 主窗口具有输入焦点时，按下的已定义功能按键或组合按键；`Command` 修饰键仅指 macOS 上的 Command 键，`Control` 修饰键仅指 Linux 上的 Control 键，`Option` 修饰键仅指 macOS 上的 Option 键，`Alt` 修饰键仅指 Linux 上的 Alt 键。
- **Undo_Keyboard_Command**：macOS 上的 `Command+Z`，或 Linux 上的 `Control+Z`。
- **Redo_Keyboard_Command**：macOS 上的 `Command+Shift+Z`，或 Linux 上的 `Control+Shift+Z`。
- **Adjustment_Increase_Keyboard_Command**：macOS 上的 `Option+Up`，或 Linux 上的 `Alt+Up`。
- **Adjustment_Decrease_Keyboard_Command**：macOS 上的 `Option+Down`，或 Linux 上的 `Alt+Down`。
- **Source_Folder**：Image_Editor 当前成功枚举并用于浏览图片的本地文件夹；Source_Folder 可由用户选择或 Startup_Folder_Planner 在启动时选择。
- **Last_Successful_Source_Folder**：最近一次由 Image_Editor 成功完成枚举并提交为 Source_Folder 的绝对路径；Image_Editor 通过 App_Settings 跨启动保存该路径。
- **Startup_Working_Directory**：Image_Editor 在本次进程启动期间获取到的初始工作目录绝对路径；后续进程工作目录变化不改变该值。
- **Startup_Folder_Planner**：按 Last_Successful_Source_Folder、Startup_Working_Directory 的优先级依次验证候选目录并产生启动枚举请求的 Image_Editor 组件。
- **Platform_Adapter**：解析平台设置存储位置、获取 Startup_Working_Directory、读取目录元数据并提供平台文件与窗口集成的边界组件。
- **Settings_Storage_Location**：由 Platform_Adapter 解析的单个 App_Settings 文件路径；macOS 使用用户 Application Support 目录，Linux 使用 `$XDG_CONFIG_HOME` 或其平台默认配置目录。
- **Sort_Field**：用于排列 Image_Collection 的字段，取值为完整文件名 `full_file_name`、最后修改时间 `modified_time` 或文件字节数 `file_size`。
- **Sort_Direction**：用于排列 Image_Collection 的方向，取值为升序 `ascending` 或降序 `descending`。
- **Sort_Settings**：由一个 Sort_Field 和一个 Sort_Direction 组成的有效排序配置；默认值为 `full_file_name` 与 `ascending`。
- **App_Settings**：Image_Editor 跨启动保存的版本化设置值，包含 Sort_Settings 和可选 Last_Successful_Source_Folder。
- **App_Settings_Store**：在 Settings_Storage_Location 对 App_Settings 执行有界读取、验证和原子替换写入的组件。
- **Effective_Sort_Settings**：启动时从有效 App_Settings 读取的 Sort_Settings；App_Settings 缺失、不可读或无效时为默认 Sort_Settings。
- **Effective_Image_Order**：先按 Effective_Sort_Settings 的 Sort_Field 和 Sort_Direction比较 Direct_Folder_File；当主字段相等时，无论 Sort_Direction 为何，均按完整文件名 UTF-8 字节序升序、再按完整本地路径 UTF-8 字节序升序打破平局；`modified_time` 或 `file_size` 元数据缺失的条目排在具有该元数据的条目之后，并使用相同平局规则。
- **Startup_Directory_Candidate**：Startup_Folder_Planner 当前验证或请求枚举的 Last_Successful_Source_Folder 或 Startup_Working_Directory。
- **Startup_Directory_Diagnostic**：标识 Startup_Directory_Candidate 类别和安全失败原因的非阻塞提示；提示不包含设置原始内容、堆栈跟踪或敏感环境值。
- **Startup_Activation_Plan**：启动目录成功产生非空 Image_Collection 后，针对 Effective_Image_Order 第一项生成的唯一自动解码计划；该计划带有对应目录枚举请求和集合修订的 revision token。
- **Direct_Folder_File**：Source_Folder 的直接子项中的本地常规文件，不包含 Source_Folder 子目录中的文件。
- **Portable_Image**：扩展名为 `.jpg`、`.jpeg`、`.png`、`.tif` 或 `.tiff` 的 Direct_Folder_File。
- **HEIC_Image**：扩展名为 `.heic` 的 Direct_Folder_File。
- **Image_Format_Capability**：Image_Editor 在当前 Supported_Platform 上可提供的特定图片格式解码或编码能力。
- **Supported_Image**：当前 Image_Format_Capability 指示可解码的 Portable_Image 或 HEIC_Image。
- **Unavailable_Image**：当前 Image_Format_Capability 指示不可解码的 Portable_Image 或 HEIC_Image。
- **Unavailable_HEIC_Image**：属于 Unavailable_Image 的 HEIC_Image。
- **Source_Image_File**：Active_Image 对应的 Supported_Image 文件。
- **Image_Collection**：Image_Editor 从一个 Source_Folder 读取的 Supported_Image 有序集合；Image_Editor 按 Effective_Image_Order 排列集合，该顺序定义每个图像的前一项、后一项、第一项和最后一项。
- **Browsing_State**：当前 Source_Folder、Image_Collection、Active_Image、每个 Active_Image 的 Edit_History 和 Redo_History，以及 Preview 内容的组合状态。
- **Preview**：Image_Editor 用于显示 Active_Image 当前编辑结果的可视区域。
- **Edit_Operation**：对 Active_Image 应用的水平翻转、垂直翻转、90 度旋转、裁剪、Brightness_Adjustment 或 Contrast_Adjustment。
- **Source_Pixel_Coordinate**：应用 Crop_Selection 时当前 Active_Image 图像数据中左上角为原点、x 轴向右、y 轴向下的整数像素坐标。
- **Source_Image_Pixel_Bounds**：当前 Active_Image 图像数据的半开矩形像素范围 `[0, W) × [0, H)`，其中 W 为像素宽度且 H 为像素高度；该范围不随 Preview 的显示缩放或窗口大小改变。
- **Crop_Selection**：由整数 Source_Pixel_Coordinate 边界 `(left, top, right, bottom)` 定义的候选矩形，其中矩形包含 `left ≤ x < right` 且 `top ≤ y < bottom` 的像素。
- **Brightness_Adjustment**：范围为 -100 至 100（含端点）的整数未提交调整值，其中 0 表示不改变 Active_Image 的亮度。
- **Contrast_Adjustment**：范围为 -100 至 100（含端点）的整数未提交调整值，其中 0 表示不改变 Active_Image 的对比度。
- **Focused_Adjustment**：当前接收 Adjustment_Increase_Keyboard_Command、Adjustment_Decrease_Keyboard_Command 和 `Return` Platform_Keyboard_Command 的 Brightness_Adjustment 或 Contrast_Adjustment。
- **Edit_History**：一个 Active_Image 在本次打开期间按应用顺序保存的 Edit_Operation 栈。
- **Redo_History**：一个 Active_Image 通过撤销操作从 Edit_History 移除、且可按后进先出顺序恢复到 Edit_History 的 Edit_Operation 栈。
- **Current_Editing_Result**：Preview 当前显示的 Active_Image 编辑结果，包括已提交的 Edit_History 和当前未提交 Focused_Adjustment 的效果。
- **Export_File**：Image_Editor 根据 Current_Editing_Result 创建的图片文件。
- **Portable_Export_Format**：JPEG、PNG 或 TIFF 格式之一。
- **Export_Format**：Portable_Export_Format，或当前 Image_Format_Capability 指示可编码的 HEIC 格式。
- **Available_Export_Format**：在当前 Supported_Platform 上具有编码 Image_Format_Capability 的 Export_Format。
- **Existing_Local_File**：用户选定导出路径上在导出请求开始前已经存在的本地常规文件。
- **Format_Appropriate_Output_Equivalence**：PNG 和 TIFF Export_File 与 Current_Editing_Result 具有相同像素宽度、高度和每个对应 RGBA 通道样本值；JPEG 和 HEIC Export_File 与 Current_Editing_Result 具有相同像素宽度、高度、方向和裁剪范围，并由对 Current_Editing_Result 的相应格式编码生成，允许该格式固有的有损压缩导致的通道样本差异。
- **Enabled_Edit_Control**：Primary_Main_Window 中可用于执行当前 Active_Image 适用 Edit_Operation 的已启用界面控件。
- **Primary_Main_Window**：Image_Editor 在一次应用启动期间用于浏览和编辑图像的唯一主窗口。
- **Conformance_Image**：像素数据、颜色空间、方向和文件内容均固定的无损 PNG 测试图像。
- **Platform_Equivalent_Result**：两个 Current_Editing_Result 具有相同像素宽度、高度、方向、裁剪范围和每个对应像素的 RGBA 通道样本值。
- **Runtime_Dependency**：Image_Editor 为提供一个或多个 Image_Format_Capability 或 Platform_Integration_Capability 而可能使用的软件组件；Runtime_Dependency 不作为启动 Primary_Main_Window 的前置条件。
- **Optional_Runtime_Dependency**：缺失时导致一个或多个 Image_Format_Capability 或 Platform_Integration_Capability 不可用、但不阻止 Image_Editor 启动的 Runtime_Dependency。
- **Installable_Platform_Package**：可在一个 Supported_Platform 上安装 Image_Editor 的分发工件，并包含或声明可用于提供 Image_Editor 能力的 Runtime_Dependency。
- **Application_Error**：Image_Editor 无法完成用户请求时产生的可显示错误状态。
- **Availability_Message**：Image_Editor 显示在 Primary_Main_Window 的相关图片列表区域或格式选择界面中的非阻塞可见状态说明。
- **Bundled_Font_Resource**：Installable_Platform_Package 内随应用分发、许可与应用分发兼容的字体资源；该资源至少包含 `U+0020–U+007E`、`U+00A0–U+00FF`、`U+2000–U+206F`、`U+2190–U+21FF`、`U+2500–U+257F`、`U+3000–U+303F`、`U+3400–U+4DBF`、`U+4E00–U+9FFF`、`U+F900–U+FAFF` 和 `U+FF00–U+FFEF` 范围内应用可能显示的字符的可辨认字形。
- **Required_Text**：Image_Editor 的中文 UI 文案、包含 Bundled_Font_Resource 覆盖范围内字符的中文文件名，以及包含这些字符的 Availability_Message 或 Application_Error。
- **Desktop_Host**：在 Supported_Platform 上创建 Primary_Main_Window、配置 egui 上下文并承载 Image_Editor 工作区的桌面应用层。
- **Font_Configuration**：Desktop_Host 在创建 Primary_Main_Window 的工作区之前构造 `egui::FontDefinitions`，将 Bundled_Font_Resource 注册为中文、常用拉丁字符和界面符号的优先字体及回退字体的配置。
- **Font_Initialization_Failure**：Bundled_Font_Resource 无法读取、字体数据无法被 Font_Configuration 接受，或 Font_Configuration 无法在首次工作区绘制前注册的状态。
- **Startup_Availability_Error**：Font_Initialization_Failure 发生时显示的安全、可见错误状态；该状态不包含堆栈跟踪、原始字体数据或其他敏感诊断数据。
- **Keybinding_Action**：Image_Editor 可由快捷键触发的具名语义命令，包括浏览、缩放与视图、编辑和文件操作；动作名称是配置文件和诊断使用的稳定 ASCII 标识符。
- **Keybinding_Gesture**：由一个受支持的非修饰键和零个或多个受支持修饰键组成的规范化按键组合；同一规范化手势在一个 Effective_Keybinding_Map 中至多属于一个 Keybinding_Action。
- **Keybinding_Configuration**：包含全局 `bindings` 表以及可选 `macos.bindings`、`linux.bindings` 表的 TOML 文档；每个声明的 Keybinding_Action 映射到一个包含一个或多个 Keybinding_Gesture 文本表示的数组。
- **Keybinding_Configuration_Parser**：将 Keybinding_Configuration 文本转换为已验证动作声明或 Keybinding_Diagnostic 的组件。
- **Keybinding_Configuration_Formatter**：将已验证 Keybinding_Configuration 转换为规范 TOML 文本的组件。
- **Keybinding_Layer**：按优先级从高到低为显式 CLI `--keybindings <path>`、项目 `.yampixr/keybindings.toml`、当前 Supported_Platform 的用户配置路径和内置默认值的一个 Keybinding_Configuration 来源。
- **Effective_Keybinding_Map**：在当前 Supported_Platform 上按 Keybinding_Layer 优先级、平台表和冲突规则解析后，从每个 Keybinding_Gesture 到至多一个 Keybinding_Action 的不可变映射。
- **Keybinding_Diagnostic**：识别 Keybinding_Layer、配置路径（内置默认值除外）、动作或手势以及失败原因的安全、可读状态；失败原因包括不可读取或无效 TOML、未知动作、未知按键、非法修饰键和重复绑定。
- **Text_Input_Focus**：Primary_Main_Window 中接收字符编辑输入并将当前 RawKeyEvent 标记为已由文本控件消费的状态。
- **View_State**：Active_Image 的当前缩放比例、适应窗口状态、画布偏移和 Preview 可用逻辑尺寸；View_State 不改变 Source_Image_File、Edit_History 或 Redo_History。
- **Zoom_Step**：将手动缩放比例乘以 `1.25` 或除以 `1.25` 的固定比例；缩放比例限制为 25% 至 800%（包含端点）。
- **Pan_Step**：按 Preview 当前可用逻辑宽度或高度的 10% 计算的画布偏移增量；画布偏移在每个轴上被限制为不显示图像边界之外的空白区域。

## Requirements

### Requirement 1: Browse Images in a Folder

**User Story:** As a macOS or Linux desktop user, I want to open a local folder and browse its pictures, so that I can choose an image to edit.

#### Acceptance Criteria

1. WHERE the Platform_Integration_Capability for selecting a local folder is available, WHEN a user invokes the open-folder action, THE Image_Editor SHALL present a Platform_File_Chooser configured to select one local folder.
2. WHEN a user selects a Source_Folder, THE Image_Editor SHALL enumerate every Direct_Folder_File, select every Supported_Image, and create the Image_Collection in Image_Collection order.
3. IF Image_Editor cannot enumerate a selected Source_Folder, THEN THE Image_Editor SHALL display an Application_Error that identifies the selected Source_Folder and retain the prior Browsing_State.
4. WHEN Image_Editor creates an Image_Collection, THE Image_Editor SHALL display every Supported_Image in the Image_Collection as a selectable entry containing the Supported_Image file name.
5. WHEN a user selects a Supported_Image entry, THE Image_Editor SHALL decode the selected Supported_Image before setting the selected Supported_Image as the Active_Image.
6. WHEN Image_Editor successfully decodes a selected Supported_Image, THE Image_Editor SHALL set the selected Supported_Image as the Active_Image and display the Active_Image in the Preview.
7. IF Image_Editor cannot decode a selected Supported_Image, THEN THE Image_Editor SHALL display an Application_Error that identifies the selected Supported_Image file name and retain the prior Browsing_State.

### Requirement 2: Keyboard-Centered Image Navigation

**User Story:** As a keyboard-oriented user, I want to move through images without using a pointer, so that I can review a folder efficiently.

#### Acceptance Criteria

1. WHEN a user presses the Right Arrow Platform_Keyboard_Command and the Active_Image has a following item in Image_Collection order, THE Image_Editor SHALL decode the following item before setting the following item as the Active_Image.
2. WHEN Image_Editor successfully decodes the following item in Image_Collection order, THE Image_Editor SHALL set the following item as the Active_Image and display the Active_Image in the Preview.
3. WHEN a user presses the Left Arrow Platform_Keyboard_Command and the Active_Image has a preceding item in Image_Collection order, THE Image_Editor SHALL decode the preceding item before setting the preceding item as the Active_Image.
4. WHEN Image_Editor successfully decodes the preceding item in Image_Collection order, THE Image_Editor SHALL set the preceding item as the Active_Image and display the Active_Image in the Preview.
5. WHEN a user presses the Home Platform_Keyboard_Command and the Image_Collection contains at least one Supported_Image, THE Image_Editor SHALL decode the first item in Image_Collection order before setting the first item as the Active_Image.
6. WHEN Image_Editor successfully decodes the first item in Image_Collection order, THE Image_Editor SHALL set the first item as the Active_Image and display the Active_Image in the Preview.
7. WHEN a user presses the End Platform_Keyboard_Command and the Image_Collection contains at least one Supported_Image, THE Image_Editor SHALL decode the last item in Image_Collection order before setting the last item as the Active_Image.
8. WHEN Image_Editor successfully decodes the last item in Image_Collection order, THE Image_Editor SHALL set the last item as the Active_Image and display the Active_Image in the Preview.
9. IF Image_Editor cannot decode a navigation candidate, THEN THE Image_Editor SHALL display an Application_Error that identifies the navigation candidate file name and retain the prior Browsing_State.
10. WHEN a user presses the Right Arrow Platform_Keyboard_Command and the Active_Image is the last item in the Image_Collection, THE Image_Editor SHALL retain the prior Browsing_State.
11. WHEN a user presses the Left Arrow Platform_Keyboard_Command and the Active_Image is the first item in the Image_Collection, THE Image_Editor SHALL retain the prior Browsing_State.
12. WHILE the Image_Collection contains no Supported_Image, WHEN a user presses the Left Arrow, Right Arrow, Home, or End Platform_Keyboard_Command, THE Image_Editor SHALL retain the prior Browsing_State and display an empty-collection message in the Preview.
13. WHILE no Active_Image exists and the Image_Collection contains one or more Supported_Image values, WHEN a user presses the Left Arrow, Right Arrow, Home, or End Platform_Keyboard_Command, THE Image_Editor SHALL retain the prior Browsing_State.

### Requirement 3: Flip and Rotate an Image

**User Story:** As an image editor, I want to flip and rotate the active image with keys, so that I can correct its orientation quickly.

#### Acceptance Criteria

1. WHEN a user presses the `F` Platform_Keyboard_Command and an Active_Image of pixel width W and pixel height H exists, THE Image_Editor SHALL append exactly one horizontal flip Edit_Operation that maps every source pixel `(x, y)` with `0 ≤ x < W` and `0 ≤ y < H` to destination pixel `(W - 1 - x, y)` and update the Preview.
2. WHEN a user presses the `Shift+F` Platform_Keyboard_Command and an Active_Image of pixel width W and pixel height H exists, THE Image_Editor SHALL append exactly one vertical flip Edit_Operation that maps every source pixel `(x, y)` with `0 ≤ x < W` and `0 ≤ y < H` to destination pixel `(x, H - 1 - y)` and update the Preview.
3. WHEN a user presses the `R` Platform_Keyboard_Command and an Active_Image of pixel width W and pixel height H exists, THE Image_Editor SHALL append exactly one 90-degree clockwise rotation Edit_Operation with destination pixel width H and destination pixel height W that maps every source pixel `(x, y)` with `0 ≤ x < W` and `0 ≤ y < H` to destination pixel `(H - 1 - y, x)` and update the Preview.
4. WHEN a user presses the `Shift+R` Platform_Keyboard_Command and an Active_Image of pixel width W and pixel height H exists, THE Image_Editor SHALL append exactly one 90-degree counterclockwise rotation Edit_Operation with destination pixel width H and destination pixel height W that maps every source pixel `(x, y)` with `0 ≤ x < W` and `0 ≤ y < H` to destination pixel `(y, W - 1 - x)` and update the Preview.
5. WHEN a user applies four consecutive 90-degree clockwise rotation Edit_Operations to an Active_Image, THE Image_Editor SHALL display the same pixel width, pixel height, and pixel locations as the Active_Image displayed before the first of those four Edit_Operations.
6. IF no Active_Image exists when Image_Editor receives the `F`, `Shift+F`, `R`, or `Shift+R` Platform_Keyboard_Command, THEN THE Image_Editor SHALL display an Application_Error and retain the Image_Collection, every Edit_History, every Redo_History, and the Preview content without executing the received Platform_Keyboard_Command after an Active_Image becomes available.

### Requirement 4: Crop an Image

**User Story:** As an image editor, I want to crop the active image, so that I can retain the intended area of the picture.

#### Acceptance Criteria

1. WHEN a user presses the `C` Platform_Keyboard_Command and an Active_Image exists, THE Image_Editor SHALL enter crop-selection mode for the Active_Image.
2. IF a user presses the `C` Platform_Keyboard_Command while no Active_Image exists, THEN THE Image_Editor SHALL display an Application_Error and retain the Image_Collection, every Edit_History, every Redo_History, and the Preview content.
3. WHILE Image_Editor is in crop-selection mode, THE Image_Editor SHALL display the Crop_Selection over the Preview using Source_Pixel_Coordinate values and Source_Image_Pixel_Bounds rather than Preview display bounds.
4. WHILE Image_Editor is in crop-selection mode, THE Image_Editor SHALL constrain every Crop_Selection boundary to an integer Source_Pixel_Coordinate within the Source_Image_Pixel_Bounds.
5. WHEN a user confirms a Crop_Selection with integer boundaries `0 ≤ left < right ≤ W` and `0 ≤ top < bottom ≤ H` in the Source_Image_Pixel_Bounds `[0, W) × [0, H)`, THE Image_Editor SHALL append exactly one crop Edit_Operation matching the Crop_Selection to the Edit_History of the Active_Image, exit crop-selection mode, and update the Preview.
6. IF a user confirms a Crop_Selection with a noninteger boundary, `left ≥ right` including `left = right`, `top ≥ bottom` including `top = bottom`, or any boundary outside the Source_Image_Pixel_Bounds, THEN THE Image_Editor SHALL display an Application_Error and retain crop-selection mode, the Crop_Selection, the Edit_History, and the Preview of the Active_Image.
7. WHEN a user cancels crop-selection mode, THE Image_Editor SHALL discard the Crop_Selection, retain the Edit_History and Preview of the Active_Image, and exit crop-selection mode.

### Requirement 5: Adjust Brightness and Contrast

**User Story:** As an image editor, I want to adjust image brightness and contrast with keys, so that I can improve image visibility without leaving the keyboard.

#### Acceptance Criteria

1. WHEN a user presses the `B` Platform_Keyboard_Command and an Active_Image exists, THE Image_Editor SHALL set the Brightness_Adjustment of the Active_Image as the Focused_Adjustment and display its effect in the Preview.
2. WHEN a user presses the `D` Platform_Keyboard_Command and an Active_Image exists, THE Image_Editor SHALL set the Contrast_Adjustment of the Active_Image as the Focused_Adjustment and display its effect in the Preview.
3. WHILE the Brightness_Adjustment is the Focused_Adjustment, WHEN a user presses the Adjustment_Increase_Keyboard_Command and the Brightness_Adjustment value is less than 100, THE Image_Editor SHALL increase the Brightness_Adjustment value by exactly 1 and display the changed value's effect in the Preview.
4. WHILE the Brightness_Adjustment is the Focused_Adjustment, WHEN a user presses the Adjustment_Decrease_Keyboard_Command and the Brightness_Adjustment value is greater than -100, THE Image_Editor SHALL decrease the Brightness_Adjustment value by exactly 1 and display the changed value's effect in the Preview.
5. WHILE the Contrast_Adjustment is the Focused_Adjustment, WHEN a user presses the Adjustment_Increase_Keyboard_Command and the Contrast_Adjustment value is less than 100, THE Image_Editor SHALL increase the Contrast_Adjustment value by exactly 1 and display the changed value's effect in the Preview.
6. WHILE the Contrast_Adjustment is the Focused_Adjustment, WHEN a user presses the Adjustment_Decrease_Keyboard_Command and the Contrast_Adjustment value is greater than -100, THE Image_Editor SHALL decrease the Contrast_Adjustment value by exactly 1 and display the changed value's effect in the Preview.
7. WHILE a Brightness_Adjustment or Contrast_Adjustment value is at 100, WHEN a user presses the Adjustment_Increase_Keyboard_Command for the Focused_Adjustment, THE Image_Editor SHALL retain the adjustment value at 100 and display the resulting Preview.
8. WHILE a Brightness_Adjustment or Contrast_Adjustment value is at -100, WHEN a user presses the Adjustment_Decrease_Keyboard_Command for the Focused_Adjustment, THE Image_Editor SHALL retain the adjustment value at -100 and display the resulting Preview.
9. WHILE the Brightness_Adjustment is the Focused_Adjustment, WHEN a user presses the `Return` Platform_Keyboard_Command, THE Image_Editor SHALL append exactly one brightness Edit_Operation containing the Brightness_Adjustment value to the Edit_History of the Active_Image, reset the Brightness_Adjustment value to 0, and update the Preview without changing the committed editing result.
10. WHILE the Contrast_Adjustment is the Focused_Adjustment, WHEN a user presses the `Return` Platform_Keyboard_Command, THE Image_Editor SHALL append exactly one contrast Edit_Operation containing the Contrast_Adjustment value to the Edit_History of the Active_Image, reset the Contrast_Adjustment value to 0, and update the Preview without changing the committed editing result.
11. WHEN the Brightness_Adjustment value and the Contrast_Adjustment value both equal 0, including after a user changes either adjustment value back to 0, THE Image_Editor SHALL display the Active_Image without a brightness or contrast change from those adjustment values.
12. IF a user presses the `B`, `D`, Adjustment_Increase_Keyboard_Command, Adjustment_Decrease_Keyboard_Command, or `Return` Platform_Keyboard_Command while no Active_Image exists, THEN THE Image_Editor SHALL display an Application_Error and retain the Image_Collection, every Edit_History, every Redo_History, and the Preview content.

### Requirement 6: Undo and Redo Edits

**User Story:** As an image editor, I want to undo and redo edits, so that I can safely compare and correct editing choices.

#### Acceptance Criteria

1. WHEN a user presses the Undo_Keyboard_Command and the Edit_History of the Active_Image contains an Edit_Operation, THE Image_Editor SHALL remove the most recently appended Edit_Operation from that Active_Image's Edit_History, append the removed Edit_Operation to that Active_Image's Redo_History, and update the Preview.
2. WHEN a user presses the Redo_Keyboard_Command and the Redo_History of the Active_Image contains an Edit_Operation, THE Image_Editor SHALL remove the most recently appended Edit_Operation from that Active_Image's Redo_History, append the removed Edit_Operation to that Active_Image's Edit_History, and update the Preview.
3. WHEN a user applies a new Edit_Operation to an Active_Image whose Redo_History contains an Edit_Operation, THE Image_Editor SHALL append the new Edit_Operation and clear only that Active_Image's Redo_History.
4. WHILE the Edit_History of the Active_Image is empty, WHEN a user presses the Undo_Keyboard_Command, THE Image_Editor SHALL retain the Edit_History, Redo_History, and Preview of the Active_Image.
5. WHILE the Redo_History of the Active_Image is empty, WHEN a user presses the Redo_Keyboard_Command, THE Image_Editor SHALL retain the Edit_History, Redo_History, and Preview of the Active_Image.
6. IF a user presses the Undo_Keyboard_Command or Redo_Keyboard_Command while no Active_Image exists, THEN THE Image_Editor SHALL retain the Image_Collection, every Edit_History, every Redo_History, and the Preview content.

### Requirement 7: Export Edited Images Without Replacing Sources

**User Story:** As an image editor, I want to export my current edits to a new file, so that the source image remains available.

#### Acceptance Criteria

1. WHERE the Platform_Integration_Capability for selecting a local export file path is available, WHEN a user invokes the export action for an Active_Image, THE Image_Editor SHALL present a Platform_File_Chooser configured to select exactly one local export file path and exactly one Available_Export_Format.
2. WHEN a user selects an Available_Export_Format and a local export file path that identifies neither the Source_Image_File of the Active_Image nor an Existing_Local_File, THE Image_Editor SHALL create exactly one Export_File at the selected export file path using the selected Available_Export_Format and the Current_Editing_Result.
3. WHEN a user selects an export file path that identifies the Source_Image_File of the Active_Image, THE Image_Editor SHALL retain the byte sequence of the Source_Image_File, every Edit_History, every Redo_History, and the Preview content.
4. WHEN a user selects an export file path that identifies an Existing_Local_File other than the Source_Image_File, THE Image_Editor SHALL retain the byte sequence of the Existing_Local_File, the Source_Image_File, every Edit_History, every Redo_History, and the Preview content.
5. WHEN Image_Editor creates an Export_File, THE Image_Editor SHALL retain the byte sequence of the Source_Image_File that existed immediately before the export operation.
6. IF Image_Editor cannot create an Export_File at the selected export file path, THEN THE Image_Editor SHALL display an Application_Error that identifies the export file path and retain the Source_Image_File, every Edit_History, every Redo_History, and the Preview content.
7. WHEN a user opens an Export_File in Image_Editor, THE Image_Editor SHALL display output that satisfies Format_Appropriate_Output_Equivalence with the Current_Editing_Result used to create the Export_File.

### Requirement 8: Provide a Focused Desktop Editing Workspace

**User Story:** As a macOS or Linux desktop user, I want a focused editing workspace, so that browsing and basic editing controls remain easy to locate.

#### Acceptance Criteria

1. WHEN Image_Editor displays an Active_Image, THE Image_Editor SHALL simultaneously display the Image_Collection, the Preview, and an Enabled_Edit_Control for every applicable horizontal flip, vertical flip, clockwise rotation, counterclockwise rotation, crop entry, crop confirmation, crop cancellation, brightness focus, contrast focus, adjustment increase, adjustment decrease, adjustment commit, undo, redo, and export operation whose required Image_Format_Capability and Platform_Integration_Capability values are available in the Primary_Main_Window.
2. WHEN a user changes the Active_Image, THE Image_Editor SHALL display the complete file name, including the extension, of the Active_Image in the Primary_Main_Window.
3. WHEN the Primary_Main_Window has input focus and a user presses a Platform_Keyboard_Command defined in this document, THE Image_Editor SHALL process the Platform_Keyboard_Command exactly once.
4. WHEN Image_Editor starts on a Supported_Platform, THE Image_Editor SHALL display exactly one Primary_Main_Window that provides the image-browsing and image-editing capabilities defined in this document.
5. WHEN Image_Editor runs on macOS, THE Image_Editor SHALL use macOS-visible names for Command and Option modifiers in every keyboard shortcut label.
6. WHEN Image_Editor runs on Linux, THE Image_Editor SHALL use Linux-visible names for Control and Alt modifiers in every keyboard shortcut label.
7. WHEN Image_Editor displays Required_Text on a Supported_Platform, THE Image_Editor SHALL render every Required_Text character covered by Bundled_Font_Resource as a recognizable character rather than a missing-glyph box.
8. WHEN Desktop_Host prepares to create the Primary_Main_Window, THE Desktop_Host SHALL register Font_Configuration through `egui::FontDefinitions` before creating the Primary_Main_Window and before Required_Text can be displayed.
9. IF Font_Initialization_Failure occurs, THEN THE Image_Editor SHALL display a Startup_Availability_Error, prevent the normal editing workspace from accepting browsing or editing commands, and terminate or remain in the Startup_Availability_Error state without rendering Required_Text with missing-glyph boxes.

### Requirement 9: Determine Available Image and Platform Capabilities

**User Story:** As a macOS or Linux desktop user, I want the application to expose only image formats and platform operations available in my current environment, so that I can make requests with predictable results.

#### Acceptance Criteria

1. WHEN Image_Editor starts on a Supported_Platform, THE Image_Editor SHALL determine the current Image_Format_Capability for JPEG, PNG, TIFF, and HEIC decoding and encoding and every Platform_Integration_Capability required by the open-folder and export operations before accepting an open-folder or export request.
2. WHEN Image_Editor completes capability determination, THE Image_Editor SHALL accept each open-folder or export request whose required Platform_Integration_Capability and Image_Format_Capability values are available.
3. WHEN Image_Editor presents a Source_Folder containing an Unavailable_Image, THE Image_Editor SHALL display an Availability_Message in the related image-list area that identifies the Unavailable_Image file name and states that decoding for the related image format is unavailable on the current Supported_Platform.
4. WHEN Image_Editor displays an Availability_Message for an Unavailable_Image, THE Image_Editor SHALL continue to provide the Primary_Main_Window and leave every Supported_Image in the Image_Collection selectable.
5. WHEN Image_Editor presents export format choices, THE Image_Editor SHALL present each Export_Format as an Available_Export_Format choice only when the current Image_Format_Capability indicates that encoding for that Export_Format is available.
6. WHEN Image_Editor determines that decoding or encoding for an Export_Format is unavailable, THE Image_Editor SHALL omit that Export_Format from the corresponding selectable image or export-format choices and display an Availability_Message that identifies the unavailable capability.
7. WHEN Image_Editor determines that a Platform_Integration_Capability required by a Dependent_Operation is unavailable, THE Image_Editor SHALL display an Availability_Message that identifies the Unavailable_Platform_Capability and disable the Dependent_Operation.
8. WHEN Image_Editor accepts a JPEG, PNG, or TIFF export request, THE Image_Editor SHALL create the Export_File without requiring HEIC decoding or HEIC encoding capability.

### Requirement 10: Preserve Equivalent Cross-Platform Editing Behavior

**User Story:** As a user who works on macOS and Linux, I want shared image processing and command behavior, so that the operating system does not change my editing outcome.

#### Acceptance Criteria

1. WHEN Image_Editor applies each Edit_Operation in the same ordered Edit_Operation sequence to the same Conformance_Image on any two Supported_Platform instances, THE Image_Editor SHALL produce Platform_Equivalent_Result values after that Edit_Operation on both instances.
2. WHEN Image_Editor exports Platform_Equivalent_Result values as PNG or TIFF on any two Supported_Platform instances whose Image_Format_Capability indicates encoding for that format is available, THE Image_Editor SHALL create Export_File values with the same pixel width, pixel height, and equal RGBA channel sample values for every corresponding pixel.
3. WHEN a user invokes a defined non-modifier Platform_Keyboard_Command on macOS or Linux, THE Image_Editor SHALL interpret the Platform_Keyboard_Command as the same named browsing or editing action on both platforms.
4. WHEN a user invokes Undo_Keyboard_Command, Redo_Keyboard_Command, Adjustment_Increase_Keyboard_Command, or Adjustment_Decrease_Keyboard_Command on the corresponding Supported_Platform, THE Image_Editor SHALL interpret the Platform_Keyboard_Command as the same named history or adjustment action on both platforms.
5. WHERE the detected Image_Format_Capability and Platform_Integration_Capability values required by the shared browsing and image-editing capabilities are available, WHEN Image_Editor starts on macOS or Linux, THE Image_Editor SHALL provide the shared browsing and image-editing capabilities defined in this document.

### Requirement 11: Provide Capability-Aware Platform Packages

**User Story:** As a macOS or Linux desktop user, I want the application to start and identify unavailable optional capabilities, so that I can use the functions supported by my current environment.

#### Acceptance Criteria

1. WHEN Image_Editor is distributed for a Supported_Platform, THE Image_Editor SHALL provide an Installable_Platform_Package that identifies the Supported_Platform and every Optional_Runtime_Dependency that provides an Image_Format_Capability or Platform_Integration_Capability.
2. WHEN Image_Editor starts and an Optional_Runtime_Dependency or Platform_Integration_Capability is unavailable, THE Image_Editor SHALL present the Primary_Main_Window, display an Availability_Message that identifies the unavailable dependency or capability, and disable every Dependent_Operation requiring the unavailable dependency or capability.
3. WHEN Image_Editor starts on a Supported_Platform and the detected capabilities required by a Dependent_Operation are available, THE Image_Editor SHALL enable the Dependent_Operation without requesting the user to install a Runtime_Dependency during that application session.
4. WHERE the Platform_Integration_Capability for selecting a local folder is available, WHEN a user invokes the open-folder action, THE Image_Editor SHALL present the corresponding Platform_File_Chooser.
5. WHERE the Platform_Integration_Capability for selecting a local export file path is available, WHEN a user invokes the export action, THE Image_Editor SHALL present the corresponding Platform_File_Chooser.
6. WHEN Image_Editor is distributed for a Supported_Platform, THE Installable_Platform_Package SHALL include a Bundled_Font_Resource and identify the Bundled_Font_Resource license and resource path in package metadata.
7. IF an Installable_Platform_Package cannot provide a readable Bundled_Font_Resource at startup, THEN THE Image_Editor SHALL produce a Startup_Availability_Error before accepting browsing or editing commands.


### Requirement 12: Configure Shortcuts and Control Image Viewing

**User Story:** As a macOS or Linux desktop user, I want to configure keyboard shortcuts and use keyboard image-view controls, so that the editor matches my workflow while retaining predictable browsing and editing behavior.

#### Acceptance Criteria

1. WHEN Image_Editor starts, THE Image_Editor SHALL build the Effective_Keybinding_Map by applying declared actions from Keybinding_Layer values in this descending order: explicit CLI `--keybindings <path>`, project `.yampixr/keybindings.toml`, macOS `~/Library/Application Support/yampixr/keybindings.toml` or Linux `$XDG_CONFIG_HOME/yampixr/keybindings.toml`, and built-in defaults; a higher Keybinding_Layer SHALL replace only the Keybinding_Action values declared by that layer.
2. WHEN Keybinding_Configuration_Parser accepts a Keybinding_Configuration, THE Keybinding_Configuration_Parser SHALL accept one or more Keybinding_Gesture values for each declared Keybinding_Action, and THE Keybinding_Configuration_Formatter SHALL render a TOML document that Keybinding_Configuration_Parser parses into an equivalent validated Keybinding_Configuration.
3. THE Image_Editor SHALL provide built-in defaults for `fit_to_window` as `0`, `zoom_actual` as `1`, `zoom_200` as `2`, `zoom_in` as `+` and `=`, `zoom_out` as `-`, `pan_left` as `H`, `pan_down` as `J`, `pan_up` as `K`, `pan_right` as `L`, `previous_image` as Left, Up, and PageUp, `next_image` as Right, Down, PageDown, and Space, `first_image` as Home, `last_image` as End, Linux `toggle_fullscreen` as F11, macOS `toggle_fullscreen` as F11 and Control+Command+F, and the existing F/Shift+F, R/Shift+R, C, B, D, Enter, platform-correct undo/redo, and platform-correct adjustment bindings.
4. IF a Keybinding_Layer cannot be read, contains invalid TOML, declares an unknown Keybinding_Action or Keybinding_Gesture key name, or declares an illegal modifier for the current Supported_Platform, THEN THE Image_Editor SHALL display a Keybinding_Diagnostic that identifies the source and reason, reject the invalid declaration, and continue resolving declarations from lower-priority Keybinding_Layer values.
5. IF two declared Keybinding_Action values normalize to the same Keybinding_Gesture in one Keybinding_Layer or a lower-priority declaration normalizes to a Keybinding_Gesture accepted from a higher-priority Keybinding_Layer, THEN THE Image_Editor SHALL display a Keybinding_Diagnostic that identifies the conflicting gesture and actions, map the Keybinding_Gesture to no more than one Keybinding_Action, and retain every non-conflicting valid declaration or lower-priority fallback declaration.
6. WHEN a user invokes `fit_to_window`, `zoom_actual`, `zoom_200`, `zoom_in`, or `zoom_out` through the Effective_Keybinding_Map while an Active_Image exists, THE Image_Editor SHALL set View_State to respectively fit the rendered Active_Image inside the Preview, exactly 100%, exactly 200%, one Zoom_Step larger, or one Zoom_Step smaller and constrain the scale to 25% through 800% inclusive.
7. WHILE the scaled Active_Image exceeds the Preview on a horizontal or vertical axis, WHEN a user invokes the corresponding `pan_left`, `pan_down`, `pan_up`, or `pan_right` Keybinding_Action, THE Image_Editor SHALL change only the applicable View_State canvas offset by one Pan_Step and constrain the canvas offset to the rendered Active_Image bounds.
8. WHILE the scaled Active_Image does not exceed the Preview on the requested pan axis, WHEN a user invokes `pan_left`, `pan_down`, `pan_up`, or `pan_right`, THE Image_Editor SHALL retain the prior View_State.
9. WHEN a user invokes any configured `previous_image`, `next_image`, `first_image`, or `last_image` Keybinding_Gesture, THE Image_Editor SHALL execute the corresponding existing navigation behavior, including collection boundaries, decode-before-activation, decode-failure retention, and no-Active_Image behavior.
10. WHEN a user invokes the configured `toggle_fullscreen` Keybinding_Gesture on macOS or Linux, THE Image_Editor SHALL request the corresponding Primary_Main_Window full-screen state transition and retain the prior Primary_Main_Window mode when the current Supported_Platform rejects the request.
11. WHEN Image_Editor displays a command control or shortcut-help entry, THE Image_Editor SHALL display every Keybinding_Gesture assigned by the Effective_Keybinding_Map using macOS Command and Option labels on macOS or Linux Control and Alt labels on Linux, and SHALL group shortcut-help entries as 浏览, 缩放与视图, 编辑, and 文件.
12. WHILE Text_Input_Focus exists, WHEN Image_Editor receives a Keybinding_Gesture marked as consumed by the focused text control, THE Image_Editor SHALL retain the Browsing_State, View_State, Edit_History, and Redo_History and allow the focused text control to process the Keybinding_Gesture.
13. WHILE no Active_Image exists, WHEN a user invokes any configured zoom, pan, or browsing Keybinding_Gesture, THE Image_Editor SHALL retain the Browsing_State and View_State.
