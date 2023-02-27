window.onload = function() {
  let button = document.getElementById("clickme");
  let ws = new WebSocket("ws://localhost:3000" + window.location.pathname + "/ws");
  ws.onopen = function() {
    console.log("Connection Made");
    button.onclick = function() {
      ws.send(JSON.stringify("EndTurn"));
      button.disabled = true;
    }
  }
  ws.onmessage = function(event) {
    let msg = JSON.parse(event.data);
    console.log(msg);
    if(msg === "YourTurn") {
      button.disabled = false;
    }
  }
}
