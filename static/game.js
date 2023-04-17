window.onload = function() {
  let end_turn_button = document.getElementById("endturn");
  let deal_button = document.getElementById("deal");
  let split_button = document.getElementById("split");
  let ws = new WebSocket("ws://localhost:3000" + window.location.pathname + "/ws");
  ws.onopen = function() {
    console.log("Connection Made");
    end_turn_button.onclick = function() {
      ws.send(JSON.stringify("EndTurn"));
      end_turn_button.disabled = true;
      deal_button.disabled = true;
    }
    deal_button.onclick = function() {
      ws.send(JSON.stringify("Deal"));
    }
    split_button.onclick = function() {
      ws.send(JSON.stringify("Split"));
      split_button.hidden = true;
      split_button.disabled = true;
    }
  }
  let player_count = 0;
  ws.onmessage = function(event) {
    let msg = JSON.parse(event.data);
    console.log(msg);
    if (msg === "EndTurn") {
      deal_button.disabled = true;
    } else if (msg === "NewHost") {
      let start_button = document.getElementById("start");
      start_button.removeAttribute("hidden");
      start_button.disabled = false;
      start_button.onclick = function() {
        ws.send(JSON.stringify("GameStart"));
        start_button.disabled = true;
        start_button.hidden = true;
      }
    } else if (msg.hasOwnProperty('YourTurn')) {
      end_turn_button.disabled = false;
      deal_button.disabled = false;
      if (msg.YourTurn.can_split) {
        console.log("You can split");
        split_button.removeAttribute("hidden");
        split_button.disabled = false;
      }
    } else if (msg.hasOwnProperty('PlayerSplit')) {
      let id = "player" + msg.PlayerSplit.player;
      document.getElementById(id + ".1").removeAttribute("hidden");
      document.getElementById(id + ".1").appendChild(document.getElementById(id).firstChild);
      //TODO: If the player leaves?
    } else if (msg.hasOwnProperty('EndGame')) {
      if (msg.EndGame.winner) {
        alert("Game has ended. You won.");
      } else {
        alert("Game has ended. You lost.");
      }
      deal_button.disabled = true;
      end_turn_button.disabled = true;
      ws.close();
      
    } else if (msg.hasOwnProperty('PlayerJoin')) {
      player_count = msg.PlayerJoin.player;
      for (let i = 0; i < msg.PlayerJoin.player; i++) {
        let player = document.getElementById("player" + i);
        player.removeAttribute("hidden");
      }
    } else if (msg.hasOwnProperty('PlayerLeave')) {

      let player_leaving = document.getElementById("player" + msg.PlayerLeave.player);
      player_leaving.innerHTML = "";
      for(let i = msg.PlayerLeave.player; i < player_count; i++) {
        let oldParent = document.getElementById("player" + (i + 1));
        let newParent = document.getElementById("player" + i);
        while(oldParent.hasChildNodes()) {
          console.log("moving card from " + oldParent.id + " to " + newParent.id);
          newParent.append(oldParent.firstChild);
        }
      }
      console.log("player_count:" + player_count);
      let player = document.getElementById("player" + (player_count - 1));
      player.setAttribute("hidden", "true");
      player_count--;
    } else if (msg.hasOwnProperty('Dealt')) {
      let card = msg.Dealt.card;
      let img = document.createElement("img");
      if (card !== null) {
        card = "" + card.rank + " of " + card.suit;
        console.log("Player " + msg.Dealt.hand + " has recieved the card " + card);
        img.src = "/static/cards/" + msg.Dealt.card.rank + msg.Dealt.card.suit + ".svg";
        
      } else {
        console.log("Player " + msg.Dealt.hand + " has recieved a card");
        img.src = "/static/cards/back.svg";
      }
      img.style = "width: 20%;";
      let id = "player" + msg.Dealt.hand;
      if (msg.Dealt.second_hand) {
        id += ".1";
      }
      document.getElementById(id).appendChild(img);
    } else if (msg.hasOwnProperty("DealDealer")) {
      let card = msg.DealDealer.card;
      let img = document.createElement("img");
      if (card !== null) {
        card = "" + card.rank + " of " + card.suit;
        console.log("Dealer has recieved the card " + card);
        img.src = "/static/cards/" + msg.DealDealer.card.rank + msg.DealDealer.card.suit + ".svg";
        
      } else {
        console.log("Dealer has recieved a card");
        img.src = "/static/cards/back.svg";
      }
      img.style = "width: 20%;";
      document.getElementById("dealer").appendChild(img);
    
    } else if (msg.hasOwnProperty('TotalHand')) {
      let id = "player" + msg.TotalHand.player;
      if(msg.TotalHand.second_hand) {
        id += ".1";
      }
      let player_cards = document.getElementById(id);
      let imgs = Array.from(player_cards.children);
      for (i in msg.TotalHand.hand) {
        let card = msg.TotalHand.hand[i];
        imgs[i].src = "/static/cards/" + card.rank + card.suit + ".svg";
      }
    } else if (msg.hasOwnProperty('TotalDealerHand')) {
      let player_cards = document.getElementById("dealer");
      let imgs = Array.from(player_cards.children);
      for (i in msg.TotalDealerHand.hand) {
        let card = msg.TotalDealerHand.hand[i];
        imgs[i].src = "/static/cards/" + card.rank + card.suit + ".svg";
      }
    }
  }
}
