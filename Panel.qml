pragma ComponentBehavior: Bound

import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "michaelkamau.eluktronics-keyboard"
  ipcTarget: "michaelkamau.eluktronics-keyboard"

  readonly property string cli: "omarchy-eluktonics-keyboard"
  readonly property var colors: [
    { name: "Red", value: "red" },
    { name: "Orange", value: "orange" },
    { name: "Yellow", value: "yellow" },
    { name: "Green", value: "green" },
    { name: "Cyan", value: "cyan" },
    { name: "Blue", value: "blue" },
    { name: "Violet", value: "violet" },
    { name: "White", value: "white" }
  ]
  readonly property var effects: ["static", "breathing", "cycle", "rainbow"]

  property bool available: false
  property bool backlightEnabled: false
  property int brightness: 0
  property int maxBrightness: 4
  property string color: "#ffffff"
  property string effect: "static"
  property string errorText: ""

  function refresh() {
    if (!statusProc.running) statusProc.running = true
  }

  function applyStatus(raw) {
    try {
      var status = JSON.parse(String(raw || "{}"))
      available = true
      backlightEnabled = status.enabled === true && status.idle_off !== true
      brightness = Number(status.brightness || 0)
      maxBrightness = Number(status.max_brightness || 4)
      color = String(status.color || "#ffffff").toLowerCase()
      effect = String(status.effect || "static")
      errorText = ""
    } catch (e) {
      available = false
      errorText = "Invalid daemon response"
    }
  }

  function run(args) {
    if (actionProc.running) return
    actionProc.command = [cli].concat(args)
    actionProc.running = true
  }

  function colorActive(value) {
    var values = {
      red: "#ff0000",
      orange: "#ffa500",
      yellow: "#ffff00",
      green: "#00ff00",
      cyan: "#00ffff",
      blue: "#0000ff",
      violet: "#8b00ff",
      white: "#ffffff"
    }
    return color === values[value]
  }

  Component.onCompleted: refresh()
  onOpenedChanged: if (opened) refresh()

  Process {
    id: statusProc
    command: [root.cli, "--json", "status"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyStatus(text)
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = String(text || "").trim()
        if (message) {
          root.available = false
          root.errorText = message
        }
      }
    }
  }

  Process {
    id: actionProc
    onExited: root.refresh()
  }

  Timer {
    interval: 5000
    running: root.opened
    repeat: true
    onTriggered: root.refresh()
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: "󰌌"
    opacity: root.available && root.backlightEnabled ? 1.0 : 0.5
    tooltipText: root.available
      ? "Keyboard RGB · " + (root.backlightEnabled ? root.effect + " · " + root.brightness + "/" + root.maxBrightness : "Off")
      : "Keyboard RGB unavailable"
    onPressed: function(b) {
      if (b === Qt.RightButton && root.available) root.run(["toggle"])
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    contentWidth: panel.fittedContentWidth(Style.space(360))
    contentHeight: panel.fittedContentHeight(content.implicitHeight)

    Column {
      id: content
      anchors.left: parent.left
      anchors.right: parent.right
      anchors.top: parent.top
      spacing: Style.space(14)

      Item {
        width: parent.width
        implicitHeight: Math.max(heroIcon.implicitHeight, heroLabels.implicitHeight, powerSwitch.implicitHeight)

        Text {
          id: heroIcon
          text: "󰌌"
          color: root.bar.foreground
          font.family: root.bar.fontFamily
          font.pixelSize: Style.font.display
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
        }

        Column {
          id: heroLabels
          anchors.left: heroIcon.right
          anchors.leftMargin: Style.space(14)
          anchors.right: powerSwitch.left
          anchors.rightMargin: Style.space(12)
          anchors.verticalCenter: parent.verticalCenter
          spacing: Style.space(2)

          Text {
            text: "Eluktronics Keyboard"
            color: root.bar.foreground
            font.family: root.bar.fontFamily
            font.pixelSize: Style.font.title
            font.bold: true
          }

          Text {
            width: parent.width
            text: root.available
              ? (root.backlightEnabled ? root.effect.toUpperCase() + " · " + root.color : "BACKLIGHT OFF")
              : "DAEMON UNAVAILABLE"
            color: Qt.darker(root.bar.foreground, 1.4)
            font.family: root.bar.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: true
            font.letterSpacing: 1.1
            elide: Text.ElideRight
          }
        }

        ToggleSwitch {
          id: powerSwitch
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          checked: root.backlightEnabled
          enabled: root.available && !actionProc.running
          busy: actionProc.running
          foreground: root.bar.foreground
          accent: Color.accent
          onToggled: root.run([root.backlightEnabled ? "off" : "on"])
        }
      }

      Text {
        visible: root.errorText !== ""
        width: parent.width
        text: root.errorText
        color: root.bar.urgent
        font.family: root.bar.fontFamily
        font.pixelSize: Style.font.bodySmall
        wrapMode: Text.WordWrap
      }

      PanelSeparator { foreground: root.bar.foreground }

      Column {
        width: parent.width
        spacing: Style.space(8)

        PanelSectionHeader {
          text: "BRIGHTNESS · " + root.brightness + "/" + root.maxBrightness
          foreground: root.bar.foreground
          fontFamily: root.bar.fontFamily
        }

        PanelSlider {
          bar: root.bar
          width: parent.width
          minimum: 0
          maximum: root.maxBrightness
          step: 1
          integer: true
          value: root.backlightEnabled ? root.brightness : 0
          enabled: root.available && !actionProc.running
          onReleased: function(v) { root.run(["brightness", String(Math.round(v))]) }
        }
      }

      PanelSeparator { foreground: root.bar.foreground }

      Column {
        width: parent.width
        spacing: Style.space(8)

        PanelSectionHeader {
          text: "COLOR"
          foreground: root.bar.foreground
          fontFamily: root.bar.fontFamily
        }

        Grid {
          id: colorGrid
          width: parent.width
          columns: 4
          spacing: Style.space(6)
          readonly property real cellWidth: (width - spacing * 3) / 4

          Repeater {
            model: root.colors
            Button {
              required property var modelData
              width: colorGrid.cellWidth
              text: modelData.name
              fontSize: Style.font.bodySmall
              foreground: root.bar.foreground
              fontFamily: root.bar.fontFamily
              bordered: true
              active: root.colorActive(modelData.value)
              enabled: root.available && !actionProc.running
              onClicked: root.run(["color", modelData.value])
            }
          }
        }
      }

      PanelSeparator { foreground: root.bar.foreground }

      Column {
        width: parent.width
        spacing: Style.space(8)

        PanelSectionHeader {
          text: "EFFECT"
          foreground: root.bar.foreground
          fontFamily: root.bar.fontFamily
        }

        Row {
          id: effectRow
          width: parent.width
          spacing: Style.space(6)
          readonly property real cellWidth: (width - spacing * 3) / 4

          Repeater {
            model: root.effects
            Button {
              required property string modelData
              width: effectRow.cellWidth
              text: modelData.charAt(0).toUpperCase() + modelData.slice(1)
              fontSize: Style.font.bodySmall
              foreground: root.bar.foreground
              fontFamily: root.bar.fontFamily
              bordered: true
              active: root.effect === modelData
              enabled: root.available && !actionProc.running
              onClicked: root.run(["effect", modelData])
            }
          }
        }
      }
    }
  }
}
