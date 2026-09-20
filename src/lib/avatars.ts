import apple from "../assets/avatars/Apple.jpg";
import artichoke from "../assets/avatars/Artichoke.jpg";
import bread from "../assets/avatars/Bread.jpg";
import cheese from "../assets/avatars/Cheese.jpg";
import eggplant from "../assets/avatars/Eggplant.jpg";
import fig from "../assets/avatars/Fig.jpg";
import garlic from "../assets/avatars/Garlic.jpg";
import grape from "../assets/avatars/Grape.jpg";
import hazelnut from "../assets/avatars/Hazelnut.jpg";
import honey from "../assets/avatars/Honey.jpg";
import lemon from "../assets/avatars/Lemon.jpg";
import mushroom from "../assets/avatars/Mushroom.jpg";
import onion from "../assets/avatars/Onion.jpg";
import orange from "../assets/avatars/Orange.jpg";
import peach from "../assets/avatars/Peach.jpg";
import pear from "../assets/avatars/Pear.jpg";
import pepper from "../assets/avatars/Pepper.jpg";
import pomegranate from "../assets/avatars/Pomegranate.jpg";
import potato from "../assets/avatars/Potato.jpg";
import pumpkin from "../assets/avatars/Pumpkin.jpg";
import quince from "../assets/avatars/Quince.jpg";
import tomato from "../assets/avatars/Tomato.jpg";
import walnut from "../assets/avatars/Walnut.jpg";

export type Avatar = {
  id: string;
  name: string;
  src: string;
};

export const AVATARS: Avatar[] = [
  { id: "apple", name: "Apple", src: apple },
  { id: "artichoke", name: "Artichoke", src: artichoke },
  { id: "bread", name: "Bread", src: bread },
  { id: "cheese", name: "Cheese", src: cheese },
  { id: "eggplant", name: "Eggplant", src: eggplant },
  { id: "fig", name: "Fig", src: fig },
  { id: "garlic", name: "Garlic", src: garlic },
  { id: "grape", name: "Grape", src: grape },
  { id: "hazelnut", name: "Hazelnut", src: hazelnut },
  { id: "honey", name: "Honey", src: honey },
  { id: "lemon", name: "Lemon", src: lemon },
  { id: "mushroom", name: "Mushroom", src: mushroom },
  { id: "onion", name: "Onion", src: onion },
  { id: "orange", name: "Orange", src: orange },
  { id: "peach", name: "Peach", src: peach },
  { id: "pear", name: "Pear", src: pear },
  { id: "pepper", name: "Pepper", src: pepper },
  { id: "pomegranate", name: "Pomegranate", src: pomegranate },
  { id: "potato", name: "Potato", src: potato },
  { id: "pumpkin", name: "Pumpkin", src: pumpkin },
  { id: "quince", name: "Quince", src: quince },
  { id: "tomato", name: "Tomato", src: tomato },
  { id: "walnut", name: "Walnut", src: walnut },
];

const byId = new Map(AVATARS.map((avatar) => [avatar.id, avatar]));

export function avatarById(id: string | undefined): Avatar | undefined {
  if (!id) return undefined;
  return byId.get(id);
}
